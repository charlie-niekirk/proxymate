curl http://localhost:8080/form \
  -X PUT \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-raw 'a=1&b=two%20words
c=$HOME `id`'
