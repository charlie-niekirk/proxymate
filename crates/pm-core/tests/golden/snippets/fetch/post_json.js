const response = await fetch("https://api.example.com/v1/notes", {
  method: "POST",
  headers: {
    "content-type": "application/json",
    "x-note": "it's \"quoted\" $HOME",
  },
  body: "{\"title\":\"Rock 'n' roll\",\"body\":\"line one\\nline two\",\"price\":\"$5\"}\n",
});
console.log(response.status);
console.log(await response.text());
