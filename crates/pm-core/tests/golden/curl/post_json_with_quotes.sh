curl https://api.example.com/v1/notes \
  -H 'content-type: application/json' \
  -H 'x-note: it'\''s "quoted"' \
  --data-raw '{"title":"Rock '\''n'\'' roll","body":"line one\nline two"}'
