import requests

response = requests.request(
    "PROPFIND",
    "https://dav.example.com/files/",
    headers={
        "depth": "1",
        "x-empty": "",
    },
)
print(response.status_code)
print(response.text)
