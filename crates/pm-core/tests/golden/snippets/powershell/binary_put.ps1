$params = @{
    Uri = 'http://localhost:8080/upload'
    Method = 'PUT'
    ContentType = 'application/octet-stream'
    Body = [Convert]::FromBase64String('AAEC//4=')
}
$response = Invoke-WebRequest @params
$response.StatusCode
$response.Content
