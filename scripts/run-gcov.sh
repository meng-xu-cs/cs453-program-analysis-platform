#!/bin/bash -e

# sanity check
if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <path-to-package-directory>"
    exit 1
fi

# configuration
PKG=$(readlink -f $1)
WKS=output-gcov

# entrypoint
cd ${PKG}
rm -rf ${WKS} && mkdir ${WKS}
gcc -fprofile-arcs -ftest-coverage -g main.c -o ${WKS}/main
for test in input/*; do
    ${WKS}/main < ${test}
done
gcov -o ${WKS} -n main.c