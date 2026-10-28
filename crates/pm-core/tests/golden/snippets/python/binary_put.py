import base64

import requests

response = requests.put(
    "http://localhost:8080/upload",
    headers={
        "content-type": "application/octet-stream",
    },
    data=base64.b64decode("AAEC//4="),
)
print(response.status_code)
print(response.text)
