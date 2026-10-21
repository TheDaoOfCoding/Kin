package app.kinrelay.linktarget;

import android.app.Activity;
import android.os.Bundle;
import android.view.Gravity;
import android.widget.TextView;

/** Shows the address it was opened with: "LINK TARGET OPENED <url>". */
public class ViewActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        TextView view = new TextView(this);
        view.setTextSize(20);
        view.setGravity(Gravity.CENTER);
        view.setPadding(48, 200, 48, 48);
        view.setText("LINK TARGET OPENED\n" + getIntent().getDataString());
        setContentView(view);
    }
}
