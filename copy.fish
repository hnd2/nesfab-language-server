set filename "nesfab-language-server"
set dst "$HOME/Library/Application Support/Zed/extensions/work/nesfab/nesfab-language-server-0.0.1/bin/$filename"

rm "$dst"
cp "target/release/$filename" $dst
zed "$HOME/Desktop/nesfab"
