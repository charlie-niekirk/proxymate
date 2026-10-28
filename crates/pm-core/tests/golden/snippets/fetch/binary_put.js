const response = await fetch("http://localhost:8080/upload", {
  method: "PUT",
  headers: {
    "content-type": "application/octet-stream",
  },
  body: Uint8Array.from(atob("AAEC//4="), (c) => c.charCodeAt(0)),
});
console.log(response.status);
console.log(await response.text());
