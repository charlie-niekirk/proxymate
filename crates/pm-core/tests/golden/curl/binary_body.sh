printf '%s' AAEC//4nCg== | base64 --decode | curl https://upload.example.com/put \
  -H 'content-type: application/octet-stream' \
  --data-binary @-
