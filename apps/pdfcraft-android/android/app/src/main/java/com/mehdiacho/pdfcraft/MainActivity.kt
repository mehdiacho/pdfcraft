package com.mehdiacho.pdfcraft

import android.app.Activity
import android.content.Intent
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.FileObserver
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.system.Os
import android.util.Log
import android.webkit.MimeTypeMap
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.google.androidgamesdk.GameActivity
import java.io.File
import java.util.concurrent.Executors

/**
 * The Activity PdfCraft runs in. The app itself is Rust (`libpdfcraft_android.so`, started by
 * android-activity on its own thread); this class does what the Rust side can't do directly:
 *
 * - the system pickers, for the app's Open, Save, Insert, Export… requests ([requestPick]).
 *   Android gives `content://` URIs, the app wants paths, so opened files are copied into the
 *   cache, and saves and exports are written to a staging folder and copied to the chosen
 *   document or folder as soon as each file is complete ([Outbox]);
 * - "Open with" and share intents, whose bytes go straight to the app's inbox.
 */
class MainActivity : GameActivity() {

    /** Copies and writes off the UI thread, one at a time, in order. */
    private val io = Executors.newSingleThreadExecutor()

    /** The picker the Rust side is waiting on (it shows one at a time). */
    private var pending: Pending? = null

    /** Staging folders being watched; kept here so they aren't collected. */
    private val outboxes = mutableListOf<Outbox>()

    private data class Pending(val id: Long, val kind: Int, val fileName: String)

    private val picker = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val p = pending ?: return@registerForActivityResult
        pending = null
        val data = result.data
        if (result.resultCode != Activity.RESULT_OK || data == null) {
            nativePickResult(p.id, emptyArray())
            return@registerForActivityResult
        }
        io.execute {
            val paths = try {
                when (p.kind) {
                    KIND_FILE, KIND_FILES -> openedUris(data).mapNotNull { copyIn(p.id, it) }
                    KIND_FOLDER -> listOfNotNull(data.data?.let { folderOutbox(p.id, it) })
                    KIND_SAVE -> listOfNotNull(data.data?.let { saveOutbox(p.id, it, p.fileName) })
                    else -> emptyList()
                }
            } catch (e: Exception) {
                Log.e(TAG, "picker result", e)
                emptyList()
            }
            nativePickResult(p.id, paths.toTypedArray())
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Before super.onCreate, which loads the Rust library: std::env::temp_dir() reads TMPDIR,
        // and Android's default (/data/local/tmp) isn't writable by apps.
        try {
            Os.setenv("TMPDIR", cacheDir.absolutePath, true)
            Os.setenv("HOME", filesDir.absolutePath, true)
        } catch (e: Exception) {
            Log.w(TAG, "couldn't set TMPDIR", e)
        }
        super.onCreate(savedInstanceState)
        keepClearOfSystemBars()
        if (savedInstanceState == null) handleIntent(intent)
    }

    /**
     * GameActivity draws edge to edge, under the status bar, navigation bar, notch and keyboard.
     * The app isn't laid out for that, so pad the surface's container by those insets: the
     * surface shrinks to the free area and the window background (the title bar's colour) fills
     * the bars. The insets still reach GameActivity, which tells the native side about them.
     */
    private fun keepClearOfSystemBars() {
        val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES
        WindowInsetsControllerCompat(window, window.decorView).apply {
            isAppearanceLightStatusBars = !night
            isAppearanceLightNavigationBars = !night
        }
        val content = findViewById<android.view.View>(android.R.id.content)
        ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
            val bars = insets.getInsets(
                WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or
                    WindowInsetsCompat.Type.ime(),
            )
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    override fun onDestroy() {
        outboxes.forEach { it.stopWatching() }
        outboxes.clear()
        super.onDestroy()
    }

    /** Called from Rust (any thread): show the system picker for request [id]. */
    @Suppress("unused")
    fun requestPick(id: Long, kind: Int, title: String, mimeTypes: Array<String>, fileName: String) {
        runOnUiThread {
            pending?.let { nativePickResult(it.id, emptyArray()) }
            val intent = when (kind) {
                KIND_FILE, KIND_FILES -> Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    setTypes(mimeTypes)
                    putExtra(Intent.EXTRA_ALLOW_MULTIPLE, kind == KIND_FILES)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                }
                KIND_FOLDER -> Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                    addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                KIND_SAVE -> Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = mimeTypes.firstOrNull()?.takeIf { it != "*/*" } ?: mimeFor(fileName)
                    putExtra(Intent.EXTRA_TITLE, fileName.ifEmpty { "document.pdf" })
                }
                else -> null
            }
            if (intent == null) {
                nativePickResult(id, emptyArray())
                return@runOnUiThread
            }
            if (title.isNotEmpty()) Log.i(TAG, "picker: $title")
            pending = Pending(id, kind, fileName)
            try {
                picker.launch(intent)
            } catch (e: Exception) {
                Log.e(TAG, "no picker for $intent", e)
                pending = null
                nativePickResult(id, emptyArray())
            }
        }
    }

    // ---- Opening ----

    private fun Intent.setTypes(mimeTypes: Array<String>) {
        if (mimeTypes.size == 1) {
            type = mimeTypes[0]
        } else {
            type = "*/*"
            if (mimeTypes.isNotEmpty() && "*/*" !in mimeTypes) putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes)
        }
    }

    private fun openedUris(data: Intent): List<Uri> {
        val clip = data.clipData
        if (clip != null) return (0 until clip.itemCount).map { clip.getItemAt(it).uri }
        return listOfNotNull(data.data)
    }

    /**
     * Copy an opened document into the cache and return its path. When the document can be
     * written, saving the copy writes the document too, so File ▸ Save saves in place.
     */
    private fun copyIn(id: Long, uri: Uri): String? {
        val writable = try {
            contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            true
        } catch (e: SecurityException) {
            false
        }
        val name = safeName(displayName(uri) ?: "document.pdf")
        val dir = File(cacheDir, "picked/$id-${System.nanoTime()}").apply { mkdirs() }
        val file = File(dir, name)
        contentResolver.openInputStream(uri)?.use { input -> file.outputStream().use { input.copyTo(it) } } ?: return null
        if (writable && supportsWrite(uri)) {
            watch(Outbox(dir) { written -> if (written.name == name) copyOut(written, uri) })
        }
        return file.absolutePath
    }

    private fun handleIntent(intent: Intent?) {
        val uris = when (intent?.action) {
            Intent.ACTION_VIEW -> listOfNotNull(intent.data)
            Intent.ACTION_SEND -> listOfNotNull(intent.streamExtra())
            Intent.ACTION_SEND_MULTIPLE -> intent.streamExtras()
            else -> emptyList()
        }
        if (uris.isEmpty()) return
        io.execute {
            for (uri in uris) {
                try {
                    val bytes = contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: continue
                    nativeOpenBytes(displayName(uri) ?: "document.pdf", bytes)
                } catch (e: Exception) {
                    Log.e(TAG, "couldn't read $uri", e)
                    toast(getString(R.string.save_failed, uri.lastPathSegment ?: "", e.message ?: ""))
                }
            }
        }
    }

    @Suppress("DEPRECATION")
    private fun Intent.streamExtra(): Uri? =
        if (Build.VERSION.SDK_INT >= 33) getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java) else getParcelableExtra(Intent.EXTRA_STREAM)

    @Suppress("DEPRECATION")
    private fun Intent.streamExtras(): List<Uri> =
        (if (Build.VERSION.SDK_INT >= 33) getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java) else getParcelableArrayListExtra(Intent.EXTRA_STREAM)) ?: emptyList()

    // ---- Saving and exporting ----

    /** Save: the app writes to a staging file; each finished write is copied to [target]. */
    private fun saveOutbox(id: Long, target: Uri, suggested: String): String {
        val name = safeName(displayName(target) ?: suggested.ifEmpty { "document.pdf" })
        val dir = File(cacheDir, "out/$id").apply { mkdirs() }
        watch(Outbox(dir) { written -> if (written.name == name) copyOut(written, target) })
        return File(dir, name).absolutePath
    }

    /** Export to a folder: every file the app finishes in the staging folder is created in [tree]. */
    private fun folderOutbox(id: Long, tree: Uri): String {
        try {
            contentResolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        } catch (e: SecurityException) {
            Log.w(TAG, "no lasting access to $tree", e)
        }
        val parent = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        val dir = File(cacheDir, "out/$id").apply { mkdirs() }
        watch(Outbox(dir) { written ->
            val doc = DocumentsContract.createDocument(contentResolver, parent, mimeFor(written.name), written.name)
            if (doc != null) copyOut(written, doc) else toast(getString(R.string.save_failed, written.name, "the folder refused the file"))
        })
        return dir.absolutePath
    }

    private fun copyOut(file: File, target: Uri) {
        try {
            contentResolver.openOutputStream(target, "wt")?.use { out -> file.inputStream().use { it.copyTo(out) } }
                ?: throw IllegalStateException("no output stream")
            toast(getString(R.string.saved_to, file.name))
        } catch (e: Exception) {
            Log.e(TAG, "couldn't write $target", e)
            toast(getString(R.string.save_failed, file.name, e.message ?: e.toString()))
        }
    }

    private fun watch(outbox: Outbox) {
        runOnUiThread {
            outboxes += outbox
            outbox.startWatching()
        }
    }

    /**
     * Watches a staging folder. The app writes files atomically (a hidden temporary file,
     * then a rename), so a finished file shows up as a rename into the folder or, for a direct
     * write, as the file being closed. Hidden files are the temporaries and are skipped.
     */
    @Suppress("DEPRECATION") // The File constructor needs API 29; minSdk is 28.
    private inner class Outbox(private val dir: File, private val onFile: (File) -> Unit) :
        FileObserver(dir.absolutePath, CLOSE_WRITE or MOVED_TO) {
        override fun onEvent(event: Int, path: String?) {
            if (path == null || path.startsWith(".")) return
            val file = File(dir, path)
            io.execute { if (file.isFile) onFile(file) }
        }
    }

    // ---- Helpers ----

    private fun displayName(uri: Uri): String? = try {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        } ?: uri.lastPathSegment?.substringAfterLast('/')
    } catch (e: Exception) {
        uri.lastPathSegment?.substringAfterLast('/')
    }

    private fun supportsWrite(uri: Uri): Boolean = try {
        contentResolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_FLAGS), null, null, null)?.use { c ->
            c.moveToFirst() && (c.getInt(0) and DocumentsContract.Document.FLAG_SUPPORTS_WRITE) != 0
        } ?: false
    } catch (e: Exception) {
        false
    }

    private fun toast(text: String) = runOnUiThread { Toast.makeText(this, text, Toast.LENGTH_SHORT).show() }

    private external fun nativeOpenBytes(name: String, bytes: ByteArray)

    private external fun nativePickResult(id: Long, paths: Array<String>)

    companion object {
        private const val TAG = "PdfCraft"

        // The kinds `requestPick` receives (apps/pdfcraft-android/src/jni_bridge.rs).
        private const val KIND_FILE = 0
        private const val KIND_FILES = 1
        private const val KIND_FOLDER = 2
        private const val KIND_SAVE = 3

        /** A file name that stays inside its folder. */
        fun safeName(name: String): String =
            name.replace('/', '_').replace('\\', '_').trimStart('.').ifEmpty { "document.pdf" }

        fun mimeFor(name: String): String =
            MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"
    }
}
