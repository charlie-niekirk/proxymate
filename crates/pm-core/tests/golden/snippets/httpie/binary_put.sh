printf %s AAEC//4= | base64 --decode | http PUT \
  http://localhost:8080/upload \
  content-type:application/octet-stream
