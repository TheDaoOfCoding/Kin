package app.kinrelay.android.link

/**
 * The order in which the link service goes down (2B.5 review round 2, B1). It is its own function so a test can hold the order: the persistent notification must stop being postable
 * **before** the runtime reports `Stopped`, because the service's collector turns that state into "Looking for <PC>" on another thread and a post that lands after the system has removed the
 * foreground notification brings it back as an ordinary ongoing one (it stayed in the shade after an unlink, 5 of 7 runs).
 */
object LinkShutdown {
    fun run(
        feed: LinkNotificationFeed?,
        stopRuntime: () -> Unit,
        markStopped: () -> Unit,
        removeNotification: () -> Unit,
    ) {
        // First: from here no post can happen, whatever the collector does with the states below.
        feed?.destroy()
        stopRuntime()
        markStopped()
        // Last: anything that was already on its way to the notification manager is taken away too.
        removeNotification()
    }
}
