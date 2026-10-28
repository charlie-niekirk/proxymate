package main

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"io"
	"net/http"
)

func main() {
	payload, err := base64.StdEncoding.DecodeString("AAEC//4=")
	if err != nil {
		panic(err)
	}
	body := bytes.NewReader(payload)
	req, err := http.NewRequest("PUT", "http://localhost:8080/upload", body)
	if err != nil {
		panic(err)
	}
	req.Header.Set("content-type", "application/octet-stream")
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
