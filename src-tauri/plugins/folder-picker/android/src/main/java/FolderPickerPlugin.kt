package app.bennovel.folderpicker

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import androidx.activity.result.ActivityResult
import app.tauri.Logger
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class ListArgs {
  lateinit var tree: String
  lateinit var id: String
}

/** Picks a folder as a document tree and lists its entries; the app walks it in Rust. */
@TauriPlugin
class FolderPickerPlugin(private val activity: Activity) : Plugin(activity) {

  @Command
  fun pickFolder(invoke: Invoke) {
    try {
      val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
      intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      startActivityForResult(invoke, intent, "folderPicked")
    } catch (ex: Exception) {
      val message = ex.message ?: "Cannot open the folder picker"
      Logger.error(message)
      invoke.reject(message)
    }
  }

  @ActivityCallback
  fun folderPicked(invoke: Invoke, result: ActivityResult) {
    try {
      val tree: Uri? = result.data?.data
      val out = JSObject()
      if (result.resultCode != Activity.RESULT_OK || tree == null) {
        invoke.resolve(out) // no "folder": cancelled
        return
      }
      val id = DocumentsContract.getTreeDocumentId(tree)
      val folder = JSObject()
      folder.put("tree", tree.toString())
      folder.put("id", id)
      folder.put("name", displayName(DocumentsContract.buildDocumentUriUsingTree(tree, id)) ?: "")
      out.put("folder", folder)
      invoke.resolve(out)
    } catch (ex: Exception) {
      val message = ex.message ?: "Cannot read the picked folder"
      Logger.error(message)
      invoke.reject(message)
    }
  }

  @Command
  fun listDir(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(ListArgs::class.java)
      val tree = Uri.parse(args.tree)
      val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, args.id)
      val columns = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE)
      val entries = JSArray()
      activity.contentResolver.query(children, columns, null, null, null)?.use { c ->
        while (c.moveToNext()) {
          val id = c.getString(0) ?: continue
          val entry = JSObject()
          entry.put("id", id)
          entry.put("name", c.getString(1) ?: "")
          entry.put("dir", c.getString(2) == Document.MIME_TYPE_DIR)
          entry.put("uri", DocumentsContract.buildDocumentUriUsingTree(tree, id).toString())
          entries.put(entry)
        }
      }
      val out = JSObject()
      out.put("entries", entries)
      invoke.resolve(out)
    } catch (ex: Exception) {
      val message = ex.message ?: "Cannot list the folder"
      Logger.error(message)
      invoke.reject(message)
    }
  }

  private fun displayName(uri: Uri): String? =
    activity.contentResolver.query(uri, arrayOf(Document.COLUMN_DISPLAY_NAME), null, null, null)?.use { c ->
      if (c.moveToFirst()) c.getString(0) else null
    }
}
