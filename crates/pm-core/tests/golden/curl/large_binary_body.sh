# The 256.0 KB request body is binary: save it as request-body.bin first (Save body to file…).
curl https://upload.example.com/big \
  --data-binary @request-body.bin
