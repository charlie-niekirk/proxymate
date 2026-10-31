const response = await fetch("https://api.example.com/v1/users?page=2&sort=name", {
  headers: {
    "accept": "application/json",
    "cookie": "session=abc; theme=dark",
    "user-agent": "Proxymate/1.0",
  },
});
console.log(response.status);
console.log(await response.text());
