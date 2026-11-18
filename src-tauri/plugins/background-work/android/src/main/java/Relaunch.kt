package app.bennovel.backgroundwork

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.Process

/**
 * Starts the app afresh, in a process of its own (`:relaunch`, see the manifest): it ends
 * the app's process, opens the screen the user asked for (with the same intent, so a file
 * or a sign-in link still arrives) and ends itself. Used when the app is opened again
 * while a translation went on with no window (TranslateService `setHeadless`): the work
 * already saved part by part carries on in the new process.
 */
class Relaunch : Activity() {
  companion object {
    private const val PID = "app.bennovel.backgroundwork.PID"
    private const val NEXT = "app.bennovel.backgroundwork.NEXT"

    /** Replaces `opened` (the screen just created in the old process) with a fresh start. */
    fun start(opened: Activity) {
      val next = Intent(opened.intent).apply {
        flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
      }
      val me = Intent(opened, Relaunch::class.java).apply {
        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        putExtra(PID, Process.myPid())
        putExtra(NEXT, next)
      }
      opened.startActivity(me)
      opened.finish()
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val pid = intent.getIntExtra(PID, -1)
    if (pid > 0) Process.killProcess(pid)
    @Suppress("DEPRECATION")
    val next = intent.getParcelableExtra<Intent>(NEXT)
    if (next != null) startActivity(next)
    finish()
    Runtime.getRuntime().exit(0)
  }
}
