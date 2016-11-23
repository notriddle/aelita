files=`find . | grep '\.\(rs\|md\|toml\|sh\|py$\)' | grep -v venv`
grep '.\{120,\}' $files | grep -v '\(http\|https\)://'
if [ $? = 0 ]; then false; else true; fi
