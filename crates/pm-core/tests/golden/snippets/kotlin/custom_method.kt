import okhttp3.OkHttpClient
import okhttp3.Request

val client = OkHttpClient()

val request = Request.Builder()
    .url("https://dav.example.com/files/")
    .method("PROPFIND", null)
    .header("depth", "1")
    .header("x-empty", "")
    .build()

client.newCall(request).execute().use { response ->
    println(response.code)
    println(response.body?.string())
}
