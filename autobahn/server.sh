cargo run --bin echo_server &

n=0
until [ $n -ge 5 ]
do
   time wstest -m fuzzingclient && break
   n=$[$n+1]
   sleep 1
done

kill %1
open reports/server/index.html