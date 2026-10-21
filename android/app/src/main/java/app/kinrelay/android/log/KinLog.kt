package app.kinrelay.android.log

/** One place for every log line. Components pass their own name; secrets are redacted on the way in. */
interface KinLog {
    fun log(level: Char, component: String, message: String)

    fun i(component: String, message: String) = log('I', component, message)
    fun w(component: String, message: String) = log('W', component, message)
    fun e(component: String, message: String) = log('E', component, message)

    object None : KinLog {
        override fun log(level: Char, component: String, message: String) = Unit
    }
}
