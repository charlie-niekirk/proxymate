$params = @{
    Uri = 'https://api.example.com/v1/notes'
    Method = 'POST'
    Headers = @{
        'x-note' = 'it''s "quoted" $HOME'
    }
    ContentType = 'application/json'
    Body = '{"title":"Rock ''n'' roll","body":"line one\nline two","price":"$5"}
'
}
$response = Invoke-WebRequest @params
$response.StatusCode
$response.Content
