$params = @{
    Uri = 'https://dav.example.com/files/'
    CustomMethod = 'PROPFIND'
    Headers = @{
        'depth' = '1'
        'x-empty' = ''
    }
}
$response = Invoke-WebRequest @params
$response.StatusCode
$response.Content
