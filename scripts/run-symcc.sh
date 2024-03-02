#!/bin/bash -e

# sanity check
if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <path-to-package-directory>"
    exit 1
fi

# configuration
PKG=$(readlink -f $1)
WKS=output-symcc
CMD=$(cat <<END
    rm -rf ${WKS} && mkdir ${WKS} &&
    /afl/afl-clang main.c -o ${WKS}/main-afl &&
    symcc main.c -o ${WKS}/main-sym &&
    screen -dmS afl -- \
        /afl/afl-fuzz -M afl-0 -i input -o ${WKS}/output -- ${WKS}/main-afl &&
    while [ ! -d "${WKS}/output" ]; do sleep 1; done &&
    symcc_fuzzing_helper -v -o ${WKS}/output -a afl-0 -n symcc -- ${WKS}/main-sym
END
)

# entrypoint
docker run \
    --tty --interactive \
    --volume ${PKG}:/test \
    --workdir /test \
    --rm symcc \
    bash -c "${CMD}"
