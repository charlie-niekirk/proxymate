package main

import (
	"fmt"
	"io"
	"net/http"
	"strings"
)

func main() {
	body := strings.NewReader("{\"title\":\"Rock 'n' roll\",\"body\":\"line one\\nline two\",\"price\":\"$5\"}\n")
	req, err := http.NewRequest("POST", "https://api.example.com/v1/notes", body)
	if err != nil {
		panic(err)
	}
	req.Header.Set("content-type", "application/json")
	req.Header.Set("x-note", "it's \"quoted\" $HOME")
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
