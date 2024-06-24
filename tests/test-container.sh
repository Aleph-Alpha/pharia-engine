#!/bin/bash

CONTAINER_NAME=$1

INTERNAL_NAME="test-container"

if [ -z $CONTAINER_NAME ]; then
    echo "missing container name."
    echo "usage: test-container.sh CONTAINER_NAME"
    exit 1
fi

if ! command -v podman &> /dev/null; then
    echo "Podman could not be found. Please install Podman."
    exit 1
fi

function time_shutdown () {
    for run in {1..$1}; do
        podman run -d -p 8081:8081 --name $INTERNAL_NAME $CONTAINER_NAME

        SECONDS=0
        START_TIME=$SECONDS
        podman stop -f name=$INTERNAL_NAME
        ELAPSED_TIME=$SECONDS

        # if the container does not shut down on SIGTERM properly,
        # podman will send SIGKILL after 10sec to force the shutdown
        if [ "$ELAPSED_TIME" -lt 10 ]; then
            return 0
        fi
        echo "retrying shutdown"
    done

    echo "shutdown time is longer than expected"
    return 1
}

time_shutdown 5
result=$?

podman rm $INTERNAL_NAME

exit $((result))
