#! /bin/bash

if [[ -z "${1}" ]]; then
    echo "Usage: ${0} <OUTPUT-FILE.tar.gz>"
    exit 1
fi

docker image ls -q --format 'table {{ .Repository }}' | grep '^mxl-example-.*$' | xargs docker save | gzip >"${1}"
