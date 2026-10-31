const response = await fetch("https://dav.example.com/files/", {
  method: "PROPFIND",
  headers: {
    "depth": "1",
    "x-empty": "",
  },
});
console.log(response.status);
console.log(await response.text());
