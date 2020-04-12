# example of how to run docker server suite
# in fuzzingclient.json config localhost need to be changed to hte host ip
# something like: "url": "ws://192.168.190.109:9001"
# and echo_server needs to be run on the same ip:
# cargo run --bin echo_server -- --bind-ip 192.168.190.109

docker run -it --rm \
    -v ${PWD}:/config \
    -v ${PWD}/reports:/reports \
    --name autobahn \
    --network="host" \
    crossbario/autobahn-testsuite \
    /usr/local/bin/wstest --mode fuzzingclient --spec /config/fuzzingclient.json
    