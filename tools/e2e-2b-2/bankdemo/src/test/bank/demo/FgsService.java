package test.bank.demo;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.IBinder;

/** A real foreground service with its notification (id 77): adb shell am start-foreground-service -n test.bank.demo/.FgsService; stop it with am stopservice. */
public class FgsService extends Service {
    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        NotificationManager nm = getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel("demo", "Demo", NotificationManager.IMPORTANCE_DEFAULT));
        Notification n = new Notification.Builder(this, "demo").setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle("ZQSkip").setContentText("ZQSkipRealFgs").build();
        startForeground(77, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE);
        return START_NOT_STICKY;
    }

    @Override
    public void onTimeout(int startId, int fgsType) {
        stopSelf();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
