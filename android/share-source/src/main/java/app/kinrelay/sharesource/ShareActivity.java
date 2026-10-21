package app.kinrelay.sharesource;

import android.app.Activity;
import android.content.ClipData;
import android.content.ComponentName;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.Process;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Random;

/** Makes n files of `size` bytes and shares them to Kin (app.kinrelay.android) with a read grant, like a gallery's Share. */
public class ShareActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        int n = getIntent().getIntExtra("n", 2);
        int size = getIntent().getIntExtra("size", 100_000);
        boolean kill = getIntent().getBooleanExtra("kill", false);
        boolean jpeg = getIntent().getBooleanExtra("jpeg", false);
        boolean chooser = getIntent().getBooleanExtra("chooser", false);
        // The smoke build of Kin has its own application ID (app.kinrelay.android.smoke); the classes keep the namespace.
        String pkg = getIntent().getStringExtra("pkg");
        if (pkg == null) pkg = "app.kinrelay.android";
        File dir = new File(getFilesDir(), "shared");
        dir.mkdirs();
        ArrayList<Uri> uris = new ArrayList<>();
        try {
            for (int i = 1; i <= n; i++) {
                String name = "shared-" + i + (jpeg ? ".jpg" : ".bin");
                byte[] bytes = new byte[size];
                new Random(i).nextBytes(bytes);
                try (FileOutputStream out = new FileOutputStream(new File(dir, name))) {
                    out.write(bytes);
                }
                uris.add(Uri.parse("content://app.kinrelay.sharesource.files/" + name));
            }
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
        Intent share = new Intent(n > 1 ? Intent.ACTION_SEND_MULTIPLE : Intent.ACTION_SEND);
        share.setType("*/*");
        if (!chooser) {
            share.setComponent(new ComponentName(pkg, "app.kinrelay.android.share.ShareActivity"));
        }
        if (n > 1) {
            share.putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris);
        } else {
            share.putExtra(Intent.EXTRA_STREAM, uris.get(0));
        }
        ClipData clip = ClipData.newRawUri("files", uris.get(0));
        for (int i = 1; i < uris.size(); i++) {
            clip.addItem(new ClipData.Item(uris.get(i)));
        }
        share.setClipData(clip);
        share.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        // chooser: the real share sheet, to see Kin and the direct-share row (the paired PC).
        startActivity(chooser ? Intent.createChooser(share, "Share") : share);
        finish();
        if (kill) {
            // The source app dies right after the share. Kin must still be able to read the files it was granted.
            new Handler(Looper.getMainLooper()).postDelayed(() -> Process.killProcess(Process.myPid()), 500);
        }
    }
}
