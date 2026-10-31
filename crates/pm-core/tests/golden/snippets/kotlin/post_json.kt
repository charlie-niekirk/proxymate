import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

val client = OkHttpClient()

val body = "{\"title\":\"Rock 'n' roll\",\"body\":\"line one\\nline two\",\"price\":\"\$5\"}\n".toRequestBody("application/json".toMediaType())
val request = Request.Builder()
    .url("https://api.example.com/v1/notes")
    .method("POST", body)
    .header("x-note", "it's \"quoted\" \$HOME")
    .build()

client.newCall(request).execute().use { response ->
    println(response.code)
    println(response.body?.string())
}
