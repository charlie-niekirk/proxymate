package main

import (
	"fmt"
	"io"
	"net/http"
)

func main() {
	req, err := http.NewRequest("GET", "https://api.example.com/v1/users?page=2&sort=name", nil)
	if err != nil {
		panic(err)
	}
	req.Header.Set("accept", "application/json")
	req.Header.Set("cookie", "session=abc; theme=dark")
	req.Header.Set("user-agent", "Proxymate/1.0")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		panic(err)
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		panic(err)
	}
	fmt.Println(resp.Status)
	fmt.Println(string(data))
}
