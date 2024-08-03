use std::{
    collections::{HashMap, HashSet},
    fs,
    io::BufRead,
    path::{Path, PathBuf},
};

use rayon::prelude::*;
use walkdir::WalkDir;

pub fn collect_cfg_map<T: AsRef<Path>>(
    files: &[T],
) -> anyhow::Result<HashMap<PathBuf, HashSet<PathBuf>>> {
    let cfg_file_paths = files
        .iter()
        .flat_map(|path| {
            WalkDir::new(path)
                .into_iter()
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let path = entry.path();
                    match (entry.file_type().is_file(), path.extension()) {
                        (true, Some(extension)) if extension == "cfg" => Some(path.to_path_buf()),
                        _ => None,
                    }
                })
        })
        .collect::<HashSet<_>>();

    let nesfab_path = Path::new(&option_env!("NESFAB").unwrap_or("")).to_path_buf();
    let cfg_map = cfg_file_paths
        .par_iter()
        .filter_map(|cfg_file_path| {
            if let Some(cfg_dir) = cfg_file_path.parent() {
                extract_inputs(&cfg_file_path)
                    .ok()
                    .map(|paths| (cfg_dir, paths))
            } else {
                None
            }
        })
        .map(|(cfg_dir, paths)| {
            (
                cfg_dir.to_path_buf(),
                paths
                    .into_iter()
                    .filter_map(|path| {
                        if let Ok(file_path) = fs::canonicalize(cfg_dir.join(&path)) {
                            return Some(file_path);
                        }
                        if let Ok(file_path) = fs::canonicalize(nesfab_path.join(&path)) {
                            return Some(file_path);
                        }
                        None
                    })
                    .filter(|path| match path.extension() {
                        Some(extension) => extension == "fab", // remove macrofab
                        None => false,
                    })
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    Ok(cfg_map)
}

fn extract_inputs<T: AsRef<Path>>(path: &T) -> anyhow::Result<Vec<PathBuf>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let paths = reader
        .lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| {
            if line.starts_with("input") {
                let parts = line.split('=').map(str::trim).collect::<Vec<_>>();
                if parts.len() == 2 {
                    let path = Path::new(parts[1]).to_path_buf();
                    return Some(path);
                }
            }
            return None;
        })
        .collect();
    Ok(paths)
}
