$params = @{
    Uri = 'https://api.example.com/v1/users?page=2&sort=name'
    Method = 'GET'
    Headers = @{
        'accept' = 'application/json'
        'cookie' = 'session=abc; theme=dark'
    }
    UserAgent = 'Proxymate/1.0'
}
$response = Invoke-WebRequest @params
$response.StatusCode
$response.Content
