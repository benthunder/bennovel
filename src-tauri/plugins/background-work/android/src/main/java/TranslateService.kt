package app.bennovel.backgroundwork

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.tauri.Logger

/**
 * Keeps the app's process (and so the AI translation running in Rust) alive while the
 * screen is off or the app is in the background, like a video player does: a foreground
 * service with an ongoing notification that shows the progress, a partial wake lock so the
 * CPU keeps running, and a Wi-Fi lock so the requests to the LLM keep their connection.
 *
 * The service is started while the app is in front (the user just asked for a
 * translation, or the app just opened and carries on its queue); Android 12+ does not let
 * an app start one from the background. Once it runs, new progress only changes its
 * notification.
 */
class TranslateService : Service() {

  /** What the notification shows; `total` 0 is an indeterminate bar. */
  data class State(
    val title: String,
    val text: String,
    val subText: String,
    val done: Int,
    val total: Int
  )

  companion object {
    private const val CHANNEL = "ai-translate"
    private const val NOTIFICATION_ID = 4107
    /** Long enough for a whole book; released before when the work ends. */
    private const val WAKE_LOCK_MS = 6L * 60 * 60 * 1000

    private val lock = Any()
    /** The running service, once it called startForeground. */
    private var running: TranslateService? = null
    /** Asked to start and not running yet. */
    private var starting = false
    /** The last state asked for. */
    private var state: State? = null

    fun show(context: Context, s: State) {
      synchronized(lock) {
        state = s
        val service = running
        if (service != null) {
          service.notifyState(s)
          return
        }
        if (starting) return
        starting = true
      }
      try {
        ContextCompat.startForegroundService(context, Intent(context, TranslateService::class.java))
      } catch (ex: Exception) {
        // Android 12+ refuses a start from the background: the translation goes on while
        // the app runs and the notification shows the next time it can start.
        synchronized(lock) { starting = false }
        Logger.error("Cannot start the translation service: ${ex.message}")
      }
    }

    fun hide(context: Context) {
      val service: TranslateService?
      synchronized(lock) {
        state = null
        service = running
      }
      // A service still starting stops itself in onStartCommand when state is null.
      service?.finish()
    }
  }

  private var wakeLock: PowerManager.WakeLock? = null
  private var wifiLock: WifiManager.WifiLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val s: State
    synchronized(lock) {
      starting = false
      // Android needs startForeground after startForegroundService even when the work
      // already ended meanwhile: start, then stop at once.
      s = state ?: State("", "", "", 0, 0)
      running = this
    }
    try {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        startForeground(NOTIFICATION_ID, build(s), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
      } else {
        startForeground(NOTIFICATION_ID, build(s))
      }
    } catch (ex: Exception) {
      Logger.error("Cannot run the translation service: ${ex.message}")
      synchronized(lock) { running = null }
      stopSelf()
      return START_NOT_STICKY
    }
    acquireLocks()
    val ended = synchronized(lock) { state == null }
    if (ended) finish()
    // Not restarted by Android after a kill: the app carries its queue on when opened.
    return START_NOT_STICKY
  }

  /** Android 15+ ends a dataSync service after 6 hours a day. */
  override fun onTimeout(startId: Int, fgsType: Int) {
    finish()
  }

  /** The app was swiped away: the queue carries on the next time it opens. */
  override fun onTaskRemoved(rootIntent: Intent?) {
    finish()
    super.onTaskRemoved(rootIntent)
  }

  override fun onDestroy() {
    synchronized(lock) {
      if (running === this) running = null
    }
    releaseLocks()
    super.onDestroy()
  }

  private fun finish() {
    synchronized(lock) {
      if (running === this) running = null
    }
    releaseLocks()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
      stopForeground(STOP_FOREGROUND_REMOVE)
    } else {
      @Suppress("DEPRECATION")
      stopForeground(true)
    }
    stopSelf()
  }

  fun notifyState(s: State) {
    try {
      val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      manager.notify(NOTIFICATION_ID, build(s))
    } catch (ex: Exception) {
      Logger.error("Cannot update the translation notification: ${ex.message}")
    }
  }

  private fun build(s: State): Notification {
    createChannel()
    val launch = packageManager.getLaunchIntentForPackage(packageName)?.apply {
      flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED
    }
    val open = launch?.let {
      PendingIntent.getActivity(
        this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
      )
    }
    val builder = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(R.drawable.ic_stat_translate)
      .setContentTitle(s.title)
      .setContentText(s.text)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .setShowWhen(false)
      .setCategory(NotificationCompat.CATEGORY_PROGRESS)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
      .setProgress(s.total.coerceAtLeast(0), s.done.coerceIn(0, s.total.coerceAtLeast(0)), s.total <= 0)
    if (s.subText.isNotEmpty()) builder.setSubText(s.subText)
    if (open != null) builder.setContentIntent(open)
    return builder.build()
  }

  private fun createChannel() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    if (manager.getNotificationChannel(CHANNEL) != null) return
    val vi = resources.configuration.locales[0].language == "vi"
    val channel = NotificationChannel(
      CHANNEL,
      if (vi) "Dịch AI chạy nền" else "AI translation in the background",
      NotificationManager.IMPORTANCE_LOW
    )
    channel.description =
      if (vi) "Tiến độ dịch khi tắt màn hình hoặc ra ngoài app" else "Translation progress while the screen is off"
    channel.setShowBadge(false)
    manager.createNotificationChannel(channel)
  }

  private fun acquireLocks() {
    try {
      if (wakeLock == null) {
        val power = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "BenNovel:translate").apply {
          setReferenceCounted(false)
          acquire(WAKE_LOCK_MS)
        }
      }
      if (wifiLock == null) {
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        // Keeps Wi-Fi on with the screen off (newer systems keep it on for a foreground
        // service anyway).
        @Suppress("DEPRECATION")
        wifiLock = wifi.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "BenNovel:translate").apply {
          setReferenceCounted(false)
          acquire()
        }
      }
    } catch (ex: Exception) {
      Logger.error("Cannot keep the device awake for the translation: ${ex.message}")
    }
  }

  private fun releaseLocks() {
    try {
      wakeLock?.let { if (it.isHeld) it.release() }
      wifiLock?.let { if (it.isHeld) it.release() }
    } catch (ex: Exception) {
      Logger.error(ex.message ?: "Cannot release the translation locks")
    }
    wakeLock = null
    wifiLock = null
  }
}
