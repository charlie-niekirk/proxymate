import Foundation

var request = URLRequest(url: URL(string: "https://dav.example.com/files/")!)
request.httpMethod = "PROPFIND"
request.setValue("1", forHTTPHeaderField: "depth")
request.setValue("", forHTTPHeaderField: "x-empty")

let (data, response) = try await URLSession.shared.data(for: request)
print((response as! HTTPURLResponse).statusCode)
print(String(decoding: data, as: UTF8.self))
