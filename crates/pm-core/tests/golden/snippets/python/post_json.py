import requests

response = requests.post(
    "https://api.example.com/v1/notes",
    headers={
        "content-type": "application/json",
        "x-note": "it's \"quoted\" $HOME",
    },
    data="{\"title\":\"Rock 'n' roll\",\"body\":\"line one\\nline two\",\"price\":\"$5\"}\n".encode(),
)
print(response.status_code)
print(response.text)
