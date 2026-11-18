package app.bennovel.backgroundwork

import android.app.Activity
import app.tauri.Logger
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class ShowArgs {
  var title: String = ""
  var text: String = ""
  var subText: String = ""
  var done: Int = 0
  var total: Int = 0
}

/** Starts, updates and stops the translation's foreground service; called from Rust. */
@TauriPlugin
class BackgroundWorkPlugin(private val activity: Activity) : Plugin(activity) {

  @Command
  fun show(invoke: Invoke) {
    try {
      val a = invoke.parseArgs(ShowArgs::class.java)
      TranslateService.show(
        activity.applicationContext,
        TranslateService.State(a.title, a.text, a.subText, a.done, a.total)
      )
      invoke.resolve(JSObject())
    } catch (ex: Exception) {
      val message = ex.message ?: "Cannot show the translation notification"
      Logger.error(message)
      invoke.reject(message)
    }
  }

  @Command
  fun hide(invoke: Invoke) {
    try {
      TranslateService.hide(activity.applicationContext)
      invoke.resolve(JSObject())
    } catch (ex: Exception) {
      val message = ex.message ?: "Cannot stop the translation service"
      Logger.error(message)
      invoke.reject(message)
    }
  }
}
