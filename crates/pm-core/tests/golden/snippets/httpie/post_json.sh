http --raw '{"title":"Rock '\''n'\'' roll","body":"line one\nline two","price":"$5"}
' \
  POST \
  https://api.example.com/v1/notes \
  content-type:application/json \
  'x-note:it'\''s "quoted" $HOME'
