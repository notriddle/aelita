files=`find . | grep '\.\(rs\|md\|toml\|sh\)'`
grep '.\{80,\}' $files | grep -v '\(http\|https\)://'
if [ $? = 0 ]; then false; else true; fi
