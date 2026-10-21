package app.kinrelay.android

import android.app.Application
import android.util.Log

class KinApp : Application() {
    lateinit var graph: AppGraph
        private set

    override fun onCreate() {
        super.onCreate()
        graph = AppGraph(this)
        graph.warmUp()
        graph.log.i("app", "process started, version ${BuildConfig.VERSION_NAME}, sdk ${android.os.Build.VERSION.SDK_INT}")
    }

    companion object {
        const val LOG_TAG = "KinRelay"
        fun mirrorToLogcat(level: Char, line: String) {
            val priority = when (level) {
                'E' -> Log.ERROR
                'W' -> Log.WARN
                else -> Log.INFO
            }
            Log.println(priority, LOG_TAG, line)
        }
    }
}

val android.content.Context.graph: AppGraph get() = (applicationContext as KinApp).graph
