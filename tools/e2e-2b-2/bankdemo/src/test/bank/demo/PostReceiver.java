package test.bank.demo;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/**
 * adb shell am broadcast -a test.bank.demo.POST -n test.bank.demo/.PostReceiver --es kind <kind> --es title <t> --es text <t> [--es tag <t>] [--ei id <n>]
 * kind: plain, call, alarm, fgs, ongoing, summary, child, secret, media, transport, progress. kind "cancel" removes tag/id.
 */
public class PostReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        NotificationManager nm = context.getSystemService(NotificationManager.class);
        String tag = intent.hasExtra("tag") ? intent.getStringExtra("tag") : "d";
        int id = intent.getIntExtra("id", 1);
        String kind = intent.hasExtra("kind") ? intent.getStringExtra("kind") : "plain";
        if (kind.equals("cancel")) {
            nm.cancel(tag, id);
            return;
        }
        nm.createNotificationChannel(new NotificationChannel("demo", "Demo", NotificationManager.IMPORTANCE_DEFAULT));
        Notification.Builder b = new Notification.Builder(context, "demo")
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle(intent.getStringExtra("title"))
                .setContentText(intent.getStringExtra("text"))
                .setAutoCancel(true);
        switch (kind) {
            case "call": b.setCategory(Notification.CATEGORY_CALL); break;
            case "alarm": b.setCategory(Notification.CATEGORY_ALARM); break;
            case "transport": b.setCategory(Notification.CATEGORY_TRANSPORT); break;
            case "ongoing": b.setOngoing(true); break;
            case "summary": b.setGroup("demo_group").setGroupSummary(true); break;
            case "child": b.setGroup("demo_group"); break;
            case "secret": b.setVisibility(Notification.VISIBILITY_SECRET); break;
            case "media": b.setStyle(new Notification.MediaStyle()); break;
            case "progress": b.setProgress(100, 40, false); break;
            default: break;
        }
        Notification n = b.build();
        if (kind.equals("fgs")) n.flags |= Notification.FLAG_FOREGROUND_SERVICE;
        nm.notify(tag, id, n);
    }
}
