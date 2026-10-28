import okhttp3.OkHttpClient
import okhttp3.Request

val client = OkHttpClient()

val request = Request.Builder()
    .url("https://api.example.com/v1/users?page=2&sort=name")
    .header("accept", "application/json")
    .header("cookie", "session=abc; theme=dark")
    .header("user-agent", "Proxymate/1.0")
    .build()

client.newCall(request).execute().use { response ->
    println(response.code)
    println(response.body?.string())
}
