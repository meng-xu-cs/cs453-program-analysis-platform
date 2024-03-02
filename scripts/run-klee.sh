#!/bin/bash -e

# sanity check
if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <path-to-package-directory>"
    exit 1
fi

# configuration
PKG=$(readlink -f $1)
WKS=output-klee
CMD=$(cat <<END
    rm -rf ${WKS} && mkdir ${WKS} &&
    clang -emit-llvm -g -O0 -c main.c -o ${WKS}/main.bc &&
    klee --libc=klee --posix-runtime \
        --output-dir=${WKS}/output \
        ${WKS}/main.bc \
        -sym-stdin 1024
END
)

# entrypoint
docker run \
    --tty --interactive \
    --ulimit='stack=-1:-1' \
    --volume ${PKG}:/test \
    --workdir /test \
    --rm klee \
    bash -c "${CMD}"
