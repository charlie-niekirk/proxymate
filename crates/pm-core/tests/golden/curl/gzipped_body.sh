printf '%s' bm90IHJlYWxseSBnemlw | base64 --decode | curl https://api.example.com/ingest \
  -H 'content-type: application/json' \
  -H 'content-encoding: gzip' \
  --data-binary @-
