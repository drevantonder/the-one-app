package app.client

import android.app.Activity
import android.app.ActivityManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Base64
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.mlkit.vision.documentscanner.GmsDocumentScannerOptions
import com.google.mlkit.vision.documentscanner.GmsDocumentScanning
import com.google.mlkit.vision.documentscanner.GmsDocumentScanningResult
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.io.ByteArrayOutputStream
import java.time.LocalDate
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

// Scanning paper for a screen (`one.scan()`), with Google's document scanner, which comes with Google Play services and
// draws its own camera: it finds each page's edges and straightens it, several in a row. Each page comes back as the
// iPhone's document camera gives it (native-apps/iphone/Sources/OneNative/Files.swift): a JPEG `{ name, type, size, base64 }`; none
// if the user cancels.
object Scanner {
    private var asked = 0

    /** Whether this phone can: Google's scanner needs Google Play services, and 1.7 GB of memory. */
    fun available(context: Context): Boolean {
        if (GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(context) != ConnectionResult.SUCCESS) return false
        val memory = ActivityManager.MemoryInfo().also { context.getSystemService(ActivityManager::class.java).getMemoryInfo(it) }
        return memory.totalMem >= 1_700_000_000L
    }

    suspend fun scan(activity: ComponentActivity): JSArray {
        if (!available(activity)) throw IllegalStateException("This device can’t scan documents. Choose the files instead.")
        val options = GmsDocumentScannerOptions.Builder()
            .setScannerMode(GmsDocumentScannerOptions.SCANNER_MODE_FULL)
            .setResultFormats(GmsDocumentScannerOptions.RESULT_FORMAT_JPEG)
            .setGalleryImportAllowed(false)
            .build()
        val start = suspendCancellableCoroutine { done ->
            GmsDocumentScanning.getClient(options).getStartScanIntent(activity)
                .addOnSuccessListener { done.resume(it) }
                .addOnFailureListener { done.resumeWithException(IllegalStateException("Couldn’t open the scanner. Choose the files instead.")) }
        }
        val result = suspendCancellableCoroutine<ActivityResult> { done ->
            lateinit var launcher: ActivityResultLauncher<IntentSenderRequest>
            launcher = activity.activityResultRegistry.register("scan-${++asked}", ActivityResultContracts.StartIntentSenderForResult()) {
                launcher.unregister()
                done.resume(it)
            }
            done.invokeOnCancellation { launcher.unregister() }
            launcher.launch(IntentSenderRequest.Builder(start).build())
        }
        if (result.resultCode != Activity.RESULT_OK) return JSArray()
        val pages = GmsDocumentScanningResult.fromActivityResultIntent(result.data)?.pages.orEmpty()
        val stamp = LocalDate.now().toString()
        return withContext(Dispatchers.IO) {
            JSArray().also { out -> pages.forEachIndexed { i, page ->
                val image = activity.contentResolver.openInputStream(page.imageUri)?.use(BitmapFactory::decodeStream) ?: return@forEachIndexed
                val jpeg = ByteArrayOutputStream().also { shrunk(image).compress(Bitmap.CompressFormat.JPEG, 80, it) }.toByteArray()
                out.put(JSObject().put("name", "Scan $stamp ${i + 1}.jpg").put("type", "image/jpeg").put("size", jpeg.size)
                    .put("base64", Base64.encodeToString(jpeg, Base64.NO_WRAP)))
            } }
        }
    }

    /** At most 2400 pixels on its long side: plenty for reading a receipt, and small enough to send. */
    private fun shrunk(image: Bitmap): Bitmap {
        val long = maxOf(image.width, image.height)
        if (long <= 2400) return image
        val scale = 2400.0 / long
        return Bitmap.createScaledBitmap(image, (image.width * scale).toInt(), (image.height * scale).toInt(), true)
    }
}
