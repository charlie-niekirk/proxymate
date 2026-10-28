import Foundation

var request = URLRequest(url: URL(string: "https://api.example.com/v1/notes")!)
request.httpMethod = "POST"
request.setValue("application/json", forHTTPHeaderField: "content-type")
request.setValue("it's \"quoted\" $HOME", forHTTPHeaderField: "x-note")
request.httpBody = Data("{\"title\":\"Rock 'n' roll\",\"body\":\"line one\\nline two\",\"price\":\"$5\"}\n".utf8)

let (data, response) = try await URLSession.shared.data(for: request)
print((response as! HTTPURLResponse).statusCode)
print(String(decoding: data, as: UTF8.self))
