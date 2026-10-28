import requests

response = requests.get(
    "https://api.example.com/v1/users?page=2&sort=name",
    headers={
        "accept": "application/json",
        "cookie": "session=abc; theme=dark",
        "user-agent": "Proxymate/1.0",
    },
)
print(response.status_code)
print(response.text)
