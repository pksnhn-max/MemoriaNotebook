package jp.memorianotebook;

import android.app.Activity;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.view.WindowInsets;

/**
 * Stage 6a vessel: one Activity, no judgement. It only forwards
 * calls to the Rust side through {@link Bridge} and shows what comes
 * back. Screens per design 2 arrive in stage 6b.
 */
public final class MainActivity extends Activity {

    private static final String SAMPLE_TEXT =
            "天満川は広島市を流れる川で、太田川の分流の一つです。"
            + "南観音は天満川の西岸にある地区で、西区に属します。"
            + "川辺では季節ごとに水位が変わり、観測の記録が残されています。"
            + "太田川は中国山地から広島湾へ注ぐ一級河川で、デルタの上に広島の市街地が広がっています。"
            + "天満川の名前は、かつて川沿いにあった天満宮に由来すると言われています。"
            + "西岸の堤防は散歩道として整備され、朝夕には歩く人や走る人の姿が見られます。"
            + "大雨の時は水位が上がるため、川辺の記録では雨量と水位を並べて残すのが習わしです。"
            + "橋は地区と対岸を結び、通勤や買い物の生活路になっています。"
            + "川の記録を続けることは、地区の変化を見守ることでもあります。";

    private TextView status;
    private TextView answer;
    private EditText question;
    private Button askButton;
    private long notebookId = -1;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        int pad = (int) (16 * getResources().getDisplayMetrics().density);
        root.setPadding(pad, pad, pad, pad);

        status = new TextView(this);
        boolean ok = false;
        try {
            ok = Bridge.init(getFilesDir().getAbsolutePath());
        } catch (Throwable t) {
            status.setText("初期化で例外: " + t);
        }
        if (ok) {
            status.setText("核の初期化 OK。まずノートと資料を作ってください。");
        } else if (status.getText().length() == 0) {
            status.setText("核の初期化に失敗しました。");
        }
        root.addView(status);

        Button setupButton = new Button(this);
        setupButton.setText("ノート作成＋資料投入");
        setupButton.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                long nb = Bridge.createNotebook("川辺の記録");
                if (nb < 0) {
                    status.setText("ノート作成に失敗しました。");
                    return;
                }
                long src = Bridge.addTextSource(nb, "天満川メモ", SAMPLE_TEXT);
                if (src < 0) {
                    status.setText("資料の投入に失敗しました。");
                    return;
                }
                notebookId = nb;
                askButton.setEnabled(true);
                status.setText("ノート " + nb + " / 資料 " + src + " を用意しました。質問できます。");
            }
        });
        root.addView(setupButton);

        question = new EditText(this);
        question.setHint("質問（例：南観音はどこにありますか）");
        root.addView(question);

        askButton = new Button(this);
        askButton.setText("質問する");
        askButton.setEnabled(false);
        askButton.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                String q = question.getText().toString();
                String a = Bridge.ask(notebookId, q);
                answer.setText(a);
            }
        });
        root.addView(askButton);

        answer = new TextView(this);
        root.addView(answer);

        ScrollView scroll = new ScrollView(this);
        scroll.addView(root);
        // targetSdk 36 draws edge-to-edge: without this, the top rows
        // of the content slide under the status bar / action bar and
        // are never visible (seen on device 2026-11-17: the status
        // line and the setup button were hidden behind the bars).
        scroll.setOnApplyWindowInsetsListener(new View.OnApplyWindowInsetsListener() {
            @Override
            public WindowInsets onApplyWindowInsets(View v, WindowInsets insets) {
                int ab = getActionBar() != null ? getActionBar().getHeight() : 0;
                v.setPadding(
                        insets.getSystemWindowInsetLeft(),
                        insets.getSystemWindowInsetTop() + ab,
                        insets.getSystemWindowInsetRight(),
                        insets.getSystemWindowInsetBottom());
                return insets;
            }
        });
        setContentView(scroll);
        // Keep the input field from grabbing focus (and panning the
        // setup rows off-screen) at launch.
        root.setFocusableInTouchMode(true);
        root.requestFocus();
    }
}
