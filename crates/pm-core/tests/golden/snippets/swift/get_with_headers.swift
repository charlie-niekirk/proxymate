import Foundation

var request = URLRequest(url: URL(string: "https://api.example.com/v1/users?page=2&sort=name")!)
request.setValue("application/json", forHTTPHeaderField: "accept")
request.setValue("session=abc; theme=dark", forHTTPHeaderField: "cookie")
request.setValue("Proxymate/1.0", forHTTPHeaderField: "user-agent")

let (data, response) = try await URLSession.shared.data(for: request)
print((response as! HTTPURLResponse).statusCode)
print(String(decoding: data, as: UTF8.self))
