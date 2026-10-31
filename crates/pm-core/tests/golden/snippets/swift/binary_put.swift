import Foundation

var request = URLRequest(url: URL(string: "http://localhost:8080/upload")!)
request.httpMethod = "PUT"
request.setValue("application/octet-stream", forHTTPHeaderField: "content-type")
request.httpBody = Data(base64Encoded: "AAEC//4=")!

let (data, response) = try await URLSession.shared.data(for: request)
print((response as! HTTPURLResponse).statusCode)
print(String(decoding: data, as: UTF8.self))
