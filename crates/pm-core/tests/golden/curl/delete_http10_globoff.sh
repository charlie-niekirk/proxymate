curl 'http://[::1]:8080/items/{id}' \
  -X DELETE \
  -H 'host: other.test' \
  --http1.0 \
  --globoff
