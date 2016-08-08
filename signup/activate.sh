#!/bin/sh
cd `dirname $0`
if [ ! -d venv/ ]; then
    virtualenv venv
    pip install -r requirements.txt
fi
source ./venv/bin/activate
source ./config.sh
export FLASK_APP=`pwd`/main.py

