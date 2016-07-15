#!/bin/sh
mkdir -p $HOME/.rust
needed_ver=`cat rust-target-version`
if [ -f $HOME/.rust/rust-target-version ]; then
  cached_ver=`cat $HOME/.rust/rust-target-version`
  if [ $cached_ver = $needed_ver ]; then
    DO_RUST=false
  else
    DO_RUST=true
  fi
else
  DO_RUST=true
fi
if [ $DO_RUST = true ]; then
  wget $needed_ver -O $HOME/.rust/rust.tar.gz
  tar -xzf $HOME/.rust/rust.tar.gz
  cd rust-nightly-*
  ./install.sh --prefix=$HOME/.rust/
  cd ..
  cp rust-target-version $HOME/.rust/
fi
export PATH="$HOME/.rust/bin:$PATH"
export LD_LIBRARY_PATH="$HOME/.rust/lib:$LD_LIBRARY_PATH"

