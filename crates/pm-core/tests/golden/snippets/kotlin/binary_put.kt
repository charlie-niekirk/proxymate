import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.Base64

val client = OkHttpClient()

val body = Base64.getDecoder().decode("AAEC//4=").toRequestBody("application/octet-stream".toMediaType())
val request = Request.Builder()
    .url("http://localhost:8080/upload")
    .method("PUT", body)
    .build()

client.newCall(request).execute().use { response ->
    println(response.code)
    println(response.body?.string())
}
