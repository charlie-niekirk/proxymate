package main

import (
	"fmt"
	"io"
	"net/http"
)

func main() {
	req, err := http.NewRequest("PROPFIND", "https://dav.example.com/files/", nil)
	if err != nil {
		panic(err)
	}
	req.Header.Set("depth", "1")
	req.Header.Set("x-empty", "")
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
