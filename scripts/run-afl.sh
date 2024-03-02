#!/bin/bash -e

# sanity check
if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <path-to-package-directory>"
    exit 1
fi

# configuration
PKG=$(readlink -f $1)
WKS=output-afl
CMD=$(cat <<END
    rm -rf ${WKS} && mkdir ${WKS} &&
    afl-cc main.c -o ${WKS}/main &&
    afl-fuzz -i input -o ${WKS}/output -- ${WKS}/main
END
)

# entrypoint
docker run \
    --tty --interactive \
    --volume ${PKG}:/test \
    --workdir /test \
    --rm afl \
    bash -c "${CMD}"
