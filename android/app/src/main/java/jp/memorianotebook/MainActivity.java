package jp.memorianotebook;
import android.widget.AutoCompleteTextView;
import android.widget.ArrayAdapter;
import org.json.JSONArray;
import org.json.JSONObject;

import android.app.Activity;
import android.content.res.Configuration;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Bundle;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.text.TextPaint;
import android.text.method.LinkMovementMethod;
import android.text.style.ClickableSpan;
import android.text.style.ReplacementSpan;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Stage 6b vessel: one Activity, no judgement. It renders the
 * design-2 screens (notebook list / sources / chat / Studio) from
 * what the Rust side returns through {@link Bridge}, and forwards
 * every action back. Screen state lives here only as "which screen
 * is showing"; all data lives in the core.
 *
 * Colours follow the design-2 tokens: dark ground #131312, surface
 * #1E1E1C, text #F2F2ED, hairline #2E2E2B; light is the same steps
 * inverted. Notebook titles are serif, UI text is sans.
 */
public final class MainActivity extends Activity {

    private boolean dark;
    private int ground, surface, ink, sub, hairline;

    private ScrollView scroll;
    private LinearLayout screen;
    private LinearLayout bottomBar;

    private long currentNb = -1;
    private String currentTitle = "";
    private int currentTab = 1; // 0 sources, 1 chat, 2 studio
    private int listFilter = 0; // 0 all, 1 mine, 2 shared, 3 downloaded
    private boolean creating = false;
    private boolean searching = false;
    private String searchQuery = "";
    private LinearLayout gridBox;

    private static final int REQ_PICK_FILE = 41;
    private final java.util.concurrent.ExecutorService bg =
            java.util.concurrent.Executors.newSingleThreadExecutor();
    private String hfQuery = "";
    private String[][] hfRepos = new String[0][];
    private String hfFileRepo = "";
    private String[][] hfFiles = new String[0][];
    private final java.util.Set<String> voiceDownloading = new java.util.HashSet<>();
    private TextView streamBubble;
    private final StringBuilder streamBuf = new StringBuilder();
    private TextView progressSink;
    private String progressSinkKey;
    private android.media.MediaPlayer player;
    private android.os.Handler playerHandler;
    private Runnable playerTick;

    /** Run JNI work that can block (generation, downloads, fetches)
     *  off the UI thread, then finish back on it. */
    private void runBg(final Runnable work, final Runnable done) {
        bg.execute(new Runnable() {
            @Override
            public void run() {
                try {
                    work.run();
                } catch (Throwable t) {
                    // JNI calls report errors as ERR: strings; a
                    // thrown exception here is a vessel bug — still
                    // finish on the UI thread so the screen recovers.
                }
                if (done != null) {
                    runOnUiThread(done);
                }
            }
        });
    }

    private void toast(String msg) {
        android.widget.Toast.makeText(this, msg,
                android.widget.Toast.LENGTH_LONG).show();
    }

    // ---------- small helpers ----------

    private int dp(int v) {
        return (int) (v * getResources().getDisplayMetrics().density);
    }

    /** Proportional scale for the top blocks (headers, chips,
     *  overview card): sizes follow the screen width as a ratio of
     *  the 400dp design width, capped so text stays readable. */
    private float uiScale() {
        float widthDp = getResources().getDisplayMetrics().widthPixels
                / getResources().getDisplayMetrics().density;
        return Math.max(1f, Math.min(1.35f, widthDp / 400f));
    }

    private int sdp(int v) {
        return dp(Math.round(v * uiScale()));
    }

    private float ssp(float sp) {
        return sp * uiScale();
    }

    private GradientDrawable cardBg() {
        GradientDrawable g = new GradientDrawable();
        g.setColor(surface);
        g.setCornerRadius(dp(14));
        g.setStroke(1, hairline);
        return g;
    }

    private TextView text(String s, float sp, int color, boolean serif, boolean bold) {
        TextView t = new TextView(this);
        t.setText(s);
        t.setTextSize(sp);
        t.setTextColor(color);
        t.setTypeface(serif ? Typeface.SERIF : Typeface.SANS_SERIF,
                bold ? Typeface.BOLD : Typeface.NORMAL);
        return t;
    }

    private LinearLayout column() {
        LinearLayout l = new LinearLayout(this);
        l.setOrientation(LinearLayout.VERTICAL);
        return l;
    }

    private LinearLayout.LayoutParams matchWrap(int topMarginDp) {
        LinearLayout.LayoutParams p = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        p.topMargin = dp(topMarginDp);
        return p;
    }

    private static String unesc(String s) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\\' && i + 1 < s.length()) {
                char n = s.charAt(++i);
                if (n == 'n') b.append('\n');
                else if (n == 't') b.append('\t');
                else b.append(n);
            } else {
                b.append(c);
            }
        }
        return b.toString();
    }

    /** Bridge errors arrive as `ERR:<kind>: <message>`. The kind is
     *  machine-readable (io, generator, notebook_not_found, ...) so
     *  the UI can branch on it; this returns just the human message
     *  for display. Older untyped `ERR: <message>` strings also parse. */
    private static String errMsg(String raw) {
        if (raw == null) return "";
        if (!raw.startsWith("ERR:")) return raw;
        String rest = raw.substring(4).trim();
        int i = rest.indexOf(": ");
        if (i > 0 && rest.substring(0, i).matches("[a-z_]+")) {
            return rest.substring(i + 2);
        }
        return rest;
    }

    private static String errKind(String raw) {
        if (raw == null || !raw.startsWith("ERR:")) return "";
        String rest = raw.substring(4).trim();
        int i = rest.indexOf(": ");
        if (i > 0 && rest.substring(0, i).matches("[a-z_]+")) {
            return rest.substring(0, i);
        }
        return "";
    }

    private static String[][] lines(String raw) {
        if (raw == null || raw.isEmpty() || raw.startsWith("ERR:")) {
            return new String[0][];
        }
        String[] ls = raw.split("\n", -1);
        List<String[]> out = new ArrayList<>();
        for (String l : ls) {
            if (!l.isEmpty()) out.add(l.split("\t", -1));
        }
        return out.toArray(new String[0][]);
    }

    private String dateOf(long unixSec) {
        if (unixSec <= 0) return "";
        return new SimpleDateFormat("yyyy年M月d日", Locale.JAPAN)
                .format(new Date(unixSec * 1000L));
    }

    // ---------- lifecycle ----------

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        int night = getResources().getConfiguration().uiMode
                & Configuration.UI_MODE_NIGHT_MASK;
        dark = night == Configuration.UI_MODE_NIGHT_YES;
        // Two colours only (ruled 2026-11-19): black and white.
        // Light mode is dark mode inverted, token for token; the
        // grey steps are gone, so secondary text and hairlines
        // resolve to the ink colour and surfaces to the ground.
        if (dark) {
            ground = Color.BLACK;
            surface = Color.BLACK;
            ink = Color.WHITE;
            sub = Color.WHITE;
            hairline = Color.WHITE;
        } else {
            ground = Color.WHITE;
            surface = Color.WHITE;
            ink = Color.BLACK;
            sub = Color.BLACK;
            hairline = Color.BLACK;
        }

        screen = column();
        screen.setBackgroundColor(ground);
        screen.setPadding(sdp(16), sdp(12), sdp(16), sdp(16));

        scroll = new ScrollView(this);
        scroll.setBackgroundColor(ground);
        scroll.addView(screen);
        scroll.setLayoutParams(new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));

        bottomBar = column();
        bottomBar.setBackgroundColor(ground);
        bottomBar.setPadding(dp(16), dp(6), dp(16), dp(10));

        LinearLayout root = column();
        root.setBackgroundColor(ground);
        root.addView(scroll);
        root.addView(bottomBar);
        root.setOnApplyWindowInsetsListener(new View.OnApplyWindowInsetsListener() {
            @Override
            public WindowInsets onApplyWindowInsets(View v, WindowInsets insets) {
                v.setPadding(
                        insets.getSystemWindowInsetLeft(),
                        insets.getSystemWindowInsetTop(),
                        insets.getSystemWindowInsetRight(),
                        insets.getSystemWindowInsetBottom());
                return insets.consumeSystemWindowInsets();
            }
        });
        setContentView(root);

        Bridge.setEventListener(new Bridge.NativeEventListener() {
            @Override
            public void onNativeEvent(final String kind, final String payload) {
                runOnUiThread(new Runnable() {
                    @Override
                    public void run() {
                        onNativeEventUi(kind, payload);
                    }
                });
            }
        });
        extractRuriAssets();
        try {
            String ortPath = getApplicationInfo().nativeLibraryDir
                    + "/libonnxruntime.so";
            if (!Bridge.init(getFilesDir().getAbsolutePath(), ortPath)) {
                screen.addView(text("核の初期化に失敗しました。", 15, ink, false, false));
                return;
            }
            seedFixtureIfPresent();
        } catch (Throwable t) {
            screen.addView(text("初期化で例外: " + t, 15, ink, false, false));
            return;
        }
        if (savedInstanceState != null
                && savedInstanceState.getLong("nb", -1) >= 0) {
            showNotebook(savedInstanceState.getLong("nb"),
                    savedInstanceState.getString("title", ""),
                    savedInstanceState.getInt("tab", 1));
        } else {
            showList();
        }
    }

    @Override
    protected void onDestroy() {
        bg.shutdownNow();
        super.onDestroy();
    }

    @Override
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        out.putLong("nb", currentNb);
        out.putString("title", currentTitle);
        out.putInt("tab", currentTab);
    }

    /** Copy the bundled ruri model files out of the APK's assets
     *  into filesDir/ruri/, where the Rust side can open them by
     *  path. Skips files already extracted at the same size. If the
     *  assets are absent (a dev build), the Rust side simply falls
     *  back to the placeholder embedder. */
    private void extractRuriAssets() {
        try {
            java.io.File dir = new java.io.File(getFilesDir(), "ruri");
            if (!dir.exists()) dir.mkdirs();
            String[] names = {"model_int8.onnx", "tokenizer.json"};
            for (String name : names) {
                java.io.File out = new java.io.File(dir, name);
                // openFd throws on compressed assets (the ruri files
                // are deflated in the APK). That must not abort the
                // extraction: fall back to "length unknown, copy".
                long assetLen = -1;
                try {
                    android.content.res.AssetFileDescriptor afd =
                            getAssets().openFd("ruri/" + name);
                    assetLen = afd.getLength();
                    afd.close();
                } catch (Throwable ignored) {
                    // Compressed asset: copy unconditionally below.
                }
                if (assetLen >= 0 && out.exists() && out.length() == assetLen) continue;
                java.io.InputStream in = getAssets().open("ruri/" + name);
                java.io.FileOutputStream fos = new java.io.FileOutputStream(out);
                byte[] buf = new byte[65536];
                int r;
                while ((r = in.read(buf)) > 0) fos.write(buf, 0, r);
                fos.close();
                in.close();
            }
        } catch (Throwable t) {
            // No ruri assets in this build: fallback seat it is.
        }
    }

    /** Events pushed from the Rust side, already on the UI thread:
     *  "token" pieces of the streaming answer, "stream_reset" when
     *  the citation gate discards a provisional attempt, and
     *  "progress" (model:<pct> / voice:<key>:<pct>) for downloads. */
    private void onNativeEventUi(String kind, String payload) {
        if ("token".equals(kind)) {
            if (streamBubble != null) {
                streamBuf.append(payload);
                streamBubble.setText(streamBuf.toString());
                scroll.fullScroll(android.view.View.FOCUS_DOWN);
            }
        } else if ("stream_reset".equals(kind)) {
            streamBuf.setLength(0);
            if (streamBubble != null) {
                streamBubble.setText("");
            }
        } else if ("progress".equals(kind)) {
            if (progressSink == null || progressSinkKey == null) return;
            int cut = payload.lastIndexOf(':');
            if (cut < 0) return;
            String head = payload.substring(0, cut);
            String pct = payload.substring(cut + 1);
            boolean match = head.equals("model")
                    ? progressSinkKey.equals("model")
                    : head.startsWith("voice:")
                            && head.substring(6).equals(progressSinkKey);
            if (match) {
                progressSink.setText("ダウンロード中… " + pct + "%");
            }
        }
    }

    private void clearScreen() {
        streamBubble = null;
        progressSink = null;
        progressSinkKey = null;
        screen.removeAllViews();
        bottomBar.removeAllViews();
        scroll.scrollTo(0, 0);
    }

    /** A pill-shaped button in the design-2 tone (no default grey slab). */
    private Button pillBtn(String label) {
        Button b = new Button(this);
        b.setText(label);
        b.setTextColor(ground);
        GradientDrawable g = new GradientDrawable();
        g.setColor(ink);
        g.setCornerRadius(dp(20));
        b.setBackground(g);
        b.setPadding(dp(16), 0, dp(16), 0);
        return b;
    }

    // ---------- nb1: notebook list ----------

    private void showList() {
        currentNb = -1;
        clearScreen();

        if (searching) {
            // Search mode: the header becomes the query field.
            LinearLayout head = new LinearLayout(this);
            head.setOrientation(LinearLayout.HORIZONTAL);
            head.setGravity(Gravity.CENTER_VERTICAL);
            final EditText q = new EditText(this);
            q.setHint("ノートを検索（題名）");
            q.setText(searchQuery);
            q.setSelection(q.getText().length());
            q.setTextColor(ink);
            q.setHintTextColor(sub);
            GradientDrawable qbg = new GradientDrawable();
            qbg.setColor(surface);
            qbg.setCornerRadius(sdp(20));
            qbg.setStroke(1, hairline);
            q.setBackground(qbg);
            q.setPadding(sdp(14), sdp(8), sdp(14), sdp(8));
            q.setLayoutParams(new LinearLayout.LayoutParams(
                    0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
            q.addTextChangedListener(new android.text.TextWatcher() {
                @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
                @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
                @Override public void afterTextChanged(android.text.Editable s) {
                    searchQuery = s.toString();
                    fillGrid();
                }
            });
            head.addView(q);
            Button done = pillBtn("やめる");
            LinearLayout.LayoutParams dp3 = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT);
            dp3.leftMargin = dp(8);
            done.setLayoutParams(dp3);
            done.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    searching = false;
                    searchQuery = "";
                    showList();
                }
            });
            head.addView(done);
            screen.addView(head);
            q.requestFocus();
        } else {
            LinearLayout head = new LinearLayout(this);
            head.setOrientation(LinearLayout.HORIZONTAL);
            head.setGravity(Gravity.CENTER_VERTICAL);
            TextView title = text("ノート", ssp(26), ink, false, true);
            title.setLayoutParams(new LinearLayout.LayoutParams(
                    0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
            head.addView(title);
            TextView searchBtn = text("⌕", ssp(22), ink, false, false);
            searchBtn.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    searching = true;
                    showList();
                }
            });
            head.addView(searchBtn);
            TextView dots = text("⋮", ssp(22), ink, false, false);
            LinearLayout.LayoutParams dp2 = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT);
            dp2.leftMargin = dp(14);
            dots.setLayoutParams(dp2);
            dots.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    openMenu();
                }
            });
            head.addView(dots);
            screen.addView(head);
        }

        // Filter chips (nb1 v3): the real product's four categories.
        // This app has no sharing, so 共有 is always empty; everything
        // is on-device, so マイノートブック / ダウンロード済み coincide
        // with すべて for now.
        LinearLayout chips = new LinearLayout(this);
        chips.setOrientation(LinearLayout.HORIZONTAL);
        String[] chipLabels = {"すべて", "マイノートブック", "共有", "ダウンロード済み"};
        for (int i = 0; i < chipLabels.length; i++) {
            final int idx = i;
            boolean on = i == listFilter;
            TextView chip = text(chipLabels[i], ssp(12.5f), on ? ground : sub, false, on);
            GradientDrawable cg = new GradientDrawable();
            cg.setColor(on ? ink : Color.TRANSPARENT);
            cg.setCornerRadius(sdp(16));
            if (!on) cg.setStroke(1, hairline);
            chip.setBackground(cg);
            chip.setPadding(sdp(11), sdp(6), sdp(11), sdp(6));
            LinearLayout.LayoutParams cp = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT);
            if (i > 0) cp.leftMargin = dp(7);
            chip.setLayoutParams(cp);
            chip.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    listFilter = idx;
                    showList();
                }
            });
            chips.addView(chip);
        }
        screen.addView(chips, matchWrap(12));

        gridBox = column();
        screen.addView(gridBox, matchWrap(0));
        fillGrid();
        buildCreateBar();
    }

    /** (Re)build only the card grid — used by search keystrokes so
     *  the query field keeps focus. */
    private void fillGrid() {
        if (gridBox == null) return;
        gridBox.removeAllViews();
        String rawList = Bridge.listNotebooks();
        if (rawList != null && rawList.startsWith("ERR:")) {
            gridBox.addView(text(rawList, 14, sub, false, false), matchWrap(14));
        }
        String[][] all = lines(rawList);
        if (listFilter == 2) {
            gridBox.addView(text("共有されたノートはありません。",
                    14, sub, false, false), matchWrap(18));
            return;
        }
        List<String[]> nbs = new ArrayList<>();
        String query = searchQuery.trim().toLowerCase(Locale.JAPAN);
        for (String[] rec : all) {
            if (query.isEmpty()
                    || (rec.length > 1
                        && unesc(rec[1]).toLowerCase(Locale.JAPAN).contains(query))) {
                nbs.add(rec);
            }
        }
        if (nbs.isEmpty()) {
            gridBox.addView(text(all.length == 0
                    ? "まだノートがありません。下の「＋ 新規作成」から作ってください。"
                    : "「" + searchQuery + "」に合うノートはありません。",
                    14, sub, false, false), matchWrap(18));
            return;
        }
        // Card grid whose column count follows the screen width:
        // a card is roughly 190dp wide, so wider screens get more
        // columns and narrow ones fewer (2 minimum, 4 maximum).
        // Cards share their row equally, so a lone card in the last
        // row stretches to the full width instead of sitting at
        // half size.
        float widthDp = getResources().getDisplayMetrics().widthPixels
                / getResources().getDisplayMetrics().density;
        int cols = Math.max(2, Math.min(4, Math.round(widthDp / 190f)));
        for (int i = 0; i < nbs.size(); i += cols) {
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.HORIZONTAL);
            for (int j = i; j < Math.min(i + cols, nbs.size()); j++) {
                row.addView(notebookCard(nbs.get(j)));
            }
            gridBox.addView(row, matchWrap(12));
        }
    }

    /** The ⋮ menu: settings (nb6 frame) and back to the top. */
    private void openMenu() {
        new android.app.AlertDialog.Builder(this)
                .setItems(new String[]{"設定", "ノート一覧へ戻る"},
                        new android.content.DialogInterface.OnClickListener() {
                            @Override
                            public void onClick(android.content.DialogInterface d, int w) {
                                if (w == 0) showSettings();
                                else showList();
                            }
                        })
                .show();
    }

    /** nb6 settings frame with today's real state: the generation
     *  seat and the local model are shown as they are; the API
     *  fields get wired when the API seat lands. */
private void showSettings() {
        searching = false;
        clearScreen();
        LinearLayout head = new LinearLayout(this);
        head.setOrientation(LinearLayout.HORIZONTAL);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView back = text("←", ssp(20), ink, false, false);
        back.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showList();
            }
        });
        head.addView(back);
        TextView t = text("設定", ssp(19), ink, false, true);
        LinearLayout.LayoutParams tp = new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f);
        tp.leftMargin = dp(10);
        t.setLayoutParams(tp);
        head.addView(t);
        screen.addView(head);

        // Load current config
        final String[] cfg = { Bridge.getApiConfig() };
        final String[] currentSeat = { getSeatFromCfg(cfg[0]) };

        screen.addView(text("生成の座", 13, sub, false, true), matchWrap(16));
        LinearLayout seat = column();
        seat.setBackground(cardBg());
        seat.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));

        final TextView localLabel = text("● ローカル（既定）", 15, ink, false, true);
        final TextView apiLabel = text("○ API（任意）", 15, ink, false, false);
        seat.addView(localLabel);
        seat.addView(text("端末内のモデルで生成します。",
                12, sub, false, false));
        seat.addView(apiLabel, matchWrap(10));
        seat.addView(text("自分のエンドポイントとキーを差す席です。",
                12, sub, false, false));
        screen.addView(seat, matchWrap(8));

        // Make seat labels clickable
        localLabel.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                currentSeat[0] = "local";
                localLabel.setText("● ローカル（既定）");
                apiLabel.setText("○ API（任意）");
            }
        });
        apiLabel.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                currentSeat[0] = "api";
                localLabel.setText("○ ローカル（既定）");
                apiLabel.setText("● API（任意）");
            }
        });

        // API config fields
        final EditText baseUrl = new EditText(this);
        baseUrl.setHint("ベース URL (例: https://api.openai.com/v1)");
        baseUrl.setTextColor(ink);
        baseUrl.setHintTextColor(sub);
        baseUrl.setBackgroundColor(Color.TRANSPARENT);
        baseUrl.setSingleLine(true);

        final EditText apiKey = new EditText(this);
        apiKey.setHint("API キー");
        apiKey.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD);
        apiKey.setTextColor(ink);
        apiKey.setHintTextColor(sub);
        apiKey.setBackgroundColor(Color.TRANSPARENT);
        apiKey.setSingleLine(true);

        final AutoCompleteTextView modelName = new AutoCompleteTextView(this);
        modelName.setHint("モデル名 (例: gpt-4o-mini)");
        modelName.setTextColor(ink);
        modelName.setHintTextColor(sub);
        modelName.setBackgroundColor(Color.TRANSPARENT);
        modelName.setSingleLine(true);
        modelName.setThreshold(0);

        final TextView status = text("", 12, sub, false, false);

        LinearLayout apiCard = column();
        apiCard.setBackground(cardBg());
        apiCard.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));
        apiCard.addView(text("API 設定", 13, sub, false, true));
        apiCard.addView(text("ベース URL", 12, sub, false, false), matchWrap(4));
        apiCard.addView(baseUrl, matchWrap(4));
        apiCard.addView(text("API キー", 12, sub, false, false), matchWrap(4));
        apiCard.addView(apiKey, matchWrap(4));
        apiCard.addView(text("モデル名", 12, sub, false, false), matchWrap(4));
        apiCard.addView(modelName, matchWrap(4));

        LinearLayout btnRow = new LinearLayout(this);
        btnRow.setOrientation(LinearLayout.HORIZONTAL);
        btnRow.setGravity(Gravity.CENTER_VERTICAL);
        Button fetchBtn = pillBtn("モデル一覧を取得");
        fetchBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                final String url = baseUrl.getText().toString().trim();
                final String key = apiKey.getText().toString().trim();
                if (url.isEmpty() || key.isEmpty()) {
                    status.setText("URL とキーを入力してください");
                    return;
                }
                fetchBtn.setEnabled(false);
                status.setText("取得中…");
                runBg(new Runnable() {
                    @Override
                    public void run() {
                        cfg[0] = Bridge.fetchApiModels(url, key);
                    }
                }, new Runnable() {
                    @Override
                    public void run() {
                        fetchBtn.setEnabled(true);
                        String res = cfg[0];
                        if (res != null && res.startsWith("[")) {
                            try {
                                org.json.JSONArray arr = new org.json.JSONArray(res);
                                List<String> models = new ArrayList<>();
                                for (int i = 0; i < arr.length(); i++) {
                                    models.add(arr.getString(i));
                                }
                                ArrayAdapter<String> adapter = new ArrayAdapter<>(
                                        MainActivity.this,
                                        android.R.layout.simple_dropdown_item_1line,
                                        models);
                                modelName.setAdapter(adapter);
                                status.setText(models.size() + " 件のモデルを取得");
                            } catch (Throwable th) {
                                status.setText("パース失敗: " + th.getMessage());
                            }
                        } else if (res != null && res.startsWith("{\"error\":")) {
                            try {
                                org.json.JSONObject obj = new org.json.JSONObject(res);
                                status.setText(obj.getString("error"));
                            } catch (Throwable th) {
                                status.setText("エラー応答");
                            }
                        } else {
                            status.setText(res != null ? res : "不明な応答");
                        }
                    }
                });
            }
        });
        btnRow.addView(fetchBtn);
        apiCard.addView(btnRow, matchWrap(8));

        Button saveBtn = pillBtn("保存");
        saveBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                final String seat = currentSeat[0];
                final String url = baseUrl.getText().toString().trim();
                final String key = apiKey.getText().toString().trim();
                final String model = modelName.getText().toString().trim();
                saveBtn.setEnabled(false);
                status.setText("保存中…");
                runBg(new Runnable() {
                    @Override
                    public void run() {
                        cfg[0] = Bridge.setApiConfig(seat, url, key, model);
                    }
                }, new Runnable() {
                    @Override
                    public void run() {
                        saveBtn.setEnabled(true);
                        status.setText(cfg[0] != null ? cfg[0] : "保存完了");
                        // Update seat labels to reflect saved seat
                        if ("api".equals(currentSeat[0])) {
                            localLabel.setText("○ ローカル（既定）");
                            apiLabel.setText("● API（任意）");
                        } else {
                            localLabel.setText("● ローカル（既定）");
                            apiLabel.setText("○ API（任意）");
                        }
                    }
                });
            }
        });
        apiCard.addView(saveBtn, matchWrap(8));
        apiCard.addView(status, matchWrap(4));
        screen.addView(apiCard, matchWrap(8));

        // Prefill from config
        prefillFromCfg(cfg[0], baseUrl, apiKey, modelName, localLabel, apiLabel);
        buildVoiceSettings();

        screen.addView(text("ローカルモデル", 13, sub, false, true), matchWrap(16));
        LinearLayout model = column();
        model.setBackground(cardBg());
        model.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));
        String modelInfo;
        try {
            modelInfo = Bridge.modelInfo();
        } catch (Throwable th) {
            modelInfo = "不明";
        }
        final boolean hasModel = modelInfo != null && !modelInfo.equals("未導入");
        model.addView(text(hasModel ? modelInfo : "TinySwallow-1.5B-Instruct　［既定］",
                15, ink, false, true));
        model.addView(text(hasModel
                        ? "導入済み（端末内）"
                        : "1.5B ・ Q5_K_M ・ 1.13 GB ・ 未導入",
                12, sub, false, false));
        String embInfo;
        try {
            embInfo = Bridge.embedderInfo();
        } catch (Throwable th) {
            embInfo = "不明";
        }
        model.addView(text("埋め込み：" + embInfo, 12, sub, false, false));
        if (!hasModel) {
            final Button dl = pillBtn("TinySwallow をダウンロード");
            dl.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    dl.setEnabled(false);
                    dl.setText("ダウンロード中…（約1.1 GB）");
                    progressSink = dl;
                    progressSinkKey = "model";
                    final String[] res = new String[1];
                    runBg(new Runnable() {
                        @Override
                        public void run() {
                            res[0] = Bridge.downloadModel(
                                    "SakanaAI/TinySwallow-1.5B-Instruct-GGUF",
                                    "tinyswallow-1.5b-instruct-q5_k_m.gguf");
                        }
                    }, new Runnable() {
                        @Override
                        public void run() {
                            if (res[0] != null && res[0].startsWith("OK")) {
                                toast("モデルを導入しました");
                            } else {
                                toast("ダウンロードに失敗しました");
                            }
                            showSettings();
                        }
                    });
                }
            });
            model.addView(dl, matchWrap(10));
        }
        screen.addView(model, matchWrap(8));

        // Model search on Hugging Face (the only add path, as in
        // Memoria's model management): search repos, pick a GGUF
        // file, download it as the local model.
        screen.addView(text("モデルを探す（Hugging Face）", 13, sub, false, true),
                matchWrap(16));
        LinearLayout hf = column();
        hf.setBackground(cardBg());
        hf.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));
        LinearLayout hfRow = new LinearLayout(this);
        hfRow.setOrientation(LinearLayout.HORIZONTAL);
        hfRow.setGravity(Gravity.CENTER_VERTICAL);
        final EditText hfQ = new EditText(this);
        hfQ.setHint("モデル名で検索");
        hfQ.setText(hfQuery);
        hfQ.setTextColor(ink);
        hfQ.setHintTextColor(sub);
        hfQ.setBackgroundColor(Color.TRANSPARENT);
        hfQ.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        hfRow.addView(hfQ);
        final Button hfGo = pillBtn("検索");
        hfGo.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                final String query = hfQ.getText().toString().trim();
                if (query.isEmpty()) return;
                hfQuery = query;
                hfGo.setEnabled(false);
                final String[] raw = new String[1];
                runBg(new Runnable() {
                    @Override
                    public void run() {
                        raw[0] = Bridge.searchModels(query);
                    }
                }, new Runnable() {
                    @Override
                    public void run() {
                        hfGo.setEnabled(true);
                        if (raw[0] != null && raw[0].startsWith("ERR:")) {
                            toast(errMsg(raw[0]));
                            return;
                        }
                        hfRepos = lines(raw[0]);
                        hfFiles = new String[0][];
                        hfFileRepo = "";
                        showSettings();
                    }
                });
            }
        });
        hfRow.addView(hfGo);
        hf.addView(hfRow);
        for (final String[] repo : hfRepos) {
            if (repo.length < 1) continue;
            TextView rv = text(repo[0], 14, ink, false, false);
            rv.setPadding(0, dp(8), 0, dp(2));
            rv.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    final String[] raw = new String[1];
                    runBg(new Runnable() {
                        @Override
                        public void run() {
                            raw[0] = Bridge.listModelFiles(repo[0]);
                        }
                    }, new Runnable() {
                        @Override
                        public void run() {
                            if (raw[0] != null && raw[0].startsWith("ERR:")) {
                                toast(errMsg(raw[0]));
                                return;
                            }
                            hfFileRepo = repo[0];
                            hfFiles = lines(raw[0]);
                            showSettings();
                        }
                    });
                }
            });
            hf.addView(rv);
        }
        if (!hfFileRepo.isEmpty()) {
            hf.addView(text(hfFileRepo + " のファイル", 12, sub, false, true),
                    matchWrap(10));
            for (final String[] f : hfFiles) {
                if (f.length < 1) continue;
                String size = "";
                if (f.length > 1) {
                    try {
                        long b = Long.parseLong(f[1]);
                        size = " ・ " + String.format(Locale.JAPAN,
                                "%.2f GB", b / 1.0e9);
                    } catch (Throwable ignored) {}
                }
                TextView fv = text(f[0] + size, 13, ink, false, false);
                fv.setPadding(0, dp(6), 0, dp(2));
                fv.setOnClickListener(new View.OnClickListener() {
                    @Override
                    public void onClick(View v) {
                        toast("ダウンロード中…: " + f[0]);
                        progressSink = fv;
                        progressSinkKey = "model";
                        final String[] res = new String[1];
                        runBg(new Runnable() {
                            @Override
                            public void run() {
                                res[0] = Bridge.downloadModel(hfFileRepo, f[0]);
                            }
                        }, new Runnable() {
                            @Override
                            public void run() {
                                toast(res[0] != null && res[0].startsWith("OK")
                                        ? "モデルを導入しました"
                                        : "ダウンロードに失敗しました");
                                showSettings();
                            }
                        });
                    }
                });
                hf.addView(fv);
            }
        }
        screen.addView(hf, matchWrap(8));

        screen.addView(text("音声概要について", 13, sub, false, true), matchWrap(16));
        LinearLayout audio = column();
        audio.setBackground(cardBg());
        audio.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));
        audio.addView(text("台本は API のモデルが書きます。ローカルモデルには書かせません。"
                + "API 未設定のとき、生成は設定への案内を出して止まります。",
                12, sub, false, false));
        screen.addView(audio, matchWrap(8));
    }

    private String getSeatFromCfg(String cfgJson) {
        if (cfgJson == null) return "local";
        try {
            org.json.JSONObject obj = new org.json.JSONObject(cfgJson);
            return obj.optString("seat", "local");
        } catch (Throwable th) {
            return "local";
        }
    }

    private void prefillFromCfg(String cfgJson, EditText baseUrl, EditText apiKey,
                                AutoCompleteTextView modelName, TextView localLabel, TextView apiLabel) {
        if (cfgJson == null) return;
        try {
            org.json.JSONObject obj = new org.json.JSONObject(cfgJson);
            String seat = obj.optString("seat", "local");
            baseUrl.setText(obj.optString("base_url", ""));
            apiKey.setText(obj.optString("api_key", ""));
            modelName.setText(obj.optString("model", ""));
            if ("api".equals(seat)) {
                localLabel.setText("○ ローカル（既定）");
                apiLabel.setText("● API（任意）");
            } else {
                localLabel.setText("● ローカル（既定）");
                apiLabel.setText("○ API（任意）");
            }
        } catch (Throwable ignored) {}
    }



    /** Bottom bar of the list screen (nb1 v3): a floating
     *  "＋ 新規作成" pill; tapped, it opens the title field in place. */
    private void buildCreateBar() {
        if (!creating) {
            LinearLayout wrap = new LinearLayout(this);
            wrap.setGravity(Gravity.CENTER);
            // The mock's floating pill is the strong tone (light in
            // dark mode), not the hairline tone of inline buttons.
            Button pill = pillBtn("＋ 新規作成");
            GradientDrawable sg = new GradientDrawable();
            sg.setColor(ink);
            sg.setCornerRadius(dp(20));
            pill.setBackground(sg);
            pill.setTextColor(ground);
            pill.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    creating = true;
                    showList();
                }
            });
            wrap.addView(pill);
            bottomBar.addView(wrap);
            return;
        }
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER_VERTICAL);
        final EditText newTitle = new EditText(this);
        newTitle.setHint("ノートのタイトル");
        newTitle.setTextColor(ink);
        newTitle.setHintTextColor(sub);
        GradientDrawable fieldBg = new GradientDrawable();
        fieldBg.setColor(surface);
        fieldBg.setCornerRadius(dp(20));
        fieldBg.setStroke(1, hairline);
        newTitle.setBackground(fieldBg);
        newTitle.setPadding(dp(14), dp(8), dp(14), dp(8));
        newTitle.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        row.addView(newTitle);
        Button createBtn = pillBtn("作成");
        LinearLayout.LayoutParams bp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        bp.leftMargin = dp(8);
        createBtn.setLayoutParams(bp);
        createBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                String t = newTitle.getText().toString().trim();
                if (t.isEmpty()) t = "無題のノート";
                creating = false;
                long id = Bridge.createNotebook(t, System.currentTimeMillis() / 1000L);
                if (id >= 0) showNotebook(id, t, 1);
                else showList();
            }
        });
        row.addView(createBtn);
        Button cancelBtn = pillBtn("やめる");
        LinearLayout.LayoutParams cp2 = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        cp2.leftMargin = dp(8);
        cancelBtn.setLayoutParams(cp2);
        cancelBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                creating = false;
                showList();
            }
        });
        row.addView(cancelBtn);
        bottomBar.addView(row);
    }

    private View notebookCard(final String[] rec) {
        final long id = Long.parseLong(rec[0]);
        final String title = rec.length > 1 ? unesc(rec[1]) : "";
        String created = rec.length > 2 ? rec[2] : "0";
        int srcCount = rec.length > 3 ? Integer.parseInt(rec[3])
                : lines(Bridge.listSources(id)).length;

        LinearLayout card = column();
        card.setBackground(cardBg());
        card.setPadding(dp(14), dp(12), dp(14), dp(12));
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f);
        lp.rightMargin = dp(6);
        lp.leftMargin = dp(6);
        card.setLayoutParams(lp);

        TextView t = text(title, 16, ink, true, true);
        card.addView(t);
        String meta = srcCount + " 件のソース"
                + (created.equals("0") ? "" : " ・ " + dateOf(Long.parseLong(created)));
        LinearLayout metaRow = new LinearLayout(this);
        metaRow.setOrientation(LinearLayout.HORIZONTAL);
        metaRow.setGravity(Gravity.CENTER_VERTICAL);
        TextView m = text(meta, 12, sub, false, false);
        m.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        metaRow.addView(m);
        // nb1 v3: the play button appears only on notebooks that
        // have an audio overview; it opens the newest one.
        final org.json.JSONObject[] latest = new org.json.JSONObject[1];
        try {
            String sums = Bridge.listVoiceSummaries(id);
            if (sums != null && !sums.startsWith("ERR:")) {
                org.json.JSONArray arr = new org.json.JSONArray(sums);
                if (arr.length() > 0) latest[0] = arr.getJSONObject(0);
            }
        } catch (Throwable ignored) {}
        if (latest[0] != null) {
            TextView play = text("▶", 15, ink, false, false);
            play.setPadding(dp(10), dp(4), dp(2), dp(4));
            play.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    currentNb = id;
                    currentTitle = title;
                    showPlayer(latest[0]);
                }
            });
            metaRow.addView(play);
        }
        LinearLayout.LayoutParams mp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        mp.topMargin = dp(18);
        metaRow.setLayoutParams(mp);
        card.addView(metaRow);
        card.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showNotebook(id, title, 1);
            }
        });
        card.setOnLongClickListener(new View.OnLongClickListener() {
            @Override
            public boolean onLongClick(View v) {
                new android.app.AlertDialog.Builder(MainActivity.this)
                        .setTitle(title)
                        .setMessage("このノートを削除しますか？（資料と履歴も一覧から消えます）")
                        .setPositiveButton("削除", new android.content.DialogInterface.OnClickListener() {
                            @Override
                            public void onClick(android.content.DialogInterface d, int w) {
                                Bridge.deleteNotebook(id);
                                showList();
                            }
                        })
                        .setNegativeButton("やめる", null)
                        .show();
                return true;
            }
        });
        return card;
    }

    // ---------- notebook shell (nb2/nb3/nb4 share the frame) ----------

    private void showNotebook(long id, String title, int tab) {
        currentNb = id;
        currentTitle = title;
        currentTab = tab;
        creating = false;
        clearScreen();

        LinearLayout head = new LinearLayout(this);
        head.setOrientation(LinearLayout.HORIZONTAL);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView back = text("←", ssp(20), ink, false, false);
        back.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showList();
            }
        });
        head.addView(back);
        TextView t = text(title, ssp(17), ink, true, true);
        LinearLayout.LayoutParams tp = new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f);
        tp.leftMargin = dp(10);
        t.setLayoutParams(tp);
        head.addView(t);
        TextView nbSearch = text("⌕", ssp(20), ink, false, false);
        nbSearch.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                searching = true;
                showList();
            }
        });
        head.addView(nbSearch);
        TextView nbDots = text("⋮", ssp(20), ink, false, false);
        LinearLayout.LayoutParams ndp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        ndp.leftMargin = dp(12);
        nbDots.setLayoutParams(ndp);
        nbDots.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                openMenu();
            }
        });
        head.addView(nbDots);
        screen.addView(head);

        LinearLayout body = column();
        screen.addView(body, matchWrap(12));
        if (tab == 0) buildSources(body);
        else if (tab == 2) buildStudio(body);
        else buildChat(body);

        bottomBar.addView(tabBar());
    }

    private View tabBar() {
        LinearLayout bar = new LinearLayout(this);
        bar.setOrientation(LinearLayout.HORIZONTAL);
        String[] labels = {"ソース", "チャット", "Studio"};
        for (int i = 0; i < labels.length; i++) {
            final int idx = i;
            TextView tv = text(labels[i], 13,
                    i == currentTab ? ground : sub, false, i == currentTab);
            tv.setGravity(Gravity.CENTER);
            if (i == currentTab) {
                GradientDrawable pill = new GradientDrawable();
                pill.setColor(ink);
                pill.setCornerRadius(dp(16));
                tv.setBackground(pill);
            }
            tv.setPadding(0, dp(8), 0, dp(8));
            tv.setLayoutParams(new LinearLayout.LayoutParams(
                    0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
            tv.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    showNotebook(currentNb, currentTitle, idx);
                }
            });
            bar.addView(tv);
        }
        return bar;
    }

    // ---------- citations ----------

    /** Small rounded citation chip drawn in place of a [#id]
     *  marker, in the mock's form: a short box carrying the
     *  citation's sequential number. The click itself is a
     *  ClickableSpan laid over the same range. */
    private final class CiteChipSpan extends ReplacementSpan {
        private final String label;

        CiteChipSpan(String label) {
            this.label = label;
        }

        private TextPaint chipPaint(Paint src) {
            TextPaint p = new TextPaint(src);
            p.setTextSize(getResources().getDisplayMetrics().scaledDensity * 10.5f);
            p.setTypeface(Typeface.create(Typeface.SANS_SERIF, Typeface.BOLD));
            p.setColor(ground);
            return p;
        }

        private int chipWidth(Paint src) {
            return Math.max(dp(18),
                    (int) Math.ceil(chipPaint(src).measureText(label)) + dp(8));
        }

        @Override
        public int getSize(Paint paint, CharSequence text, int start,
                           int end, Paint.FontMetricsInt fm) {
            if (fm != null) {
                fm.ascent = Math.min(fm.ascent, -dp(15));
                fm.descent = Math.max(fm.descent, dp(3));
                fm.top = Math.min(fm.top, -dp(15));
                fm.bottom = Math.max(fm.bottom, dp(3));
            }
            return chipWidth(paint) + dp(2);
        }

        @Override
        public void draw(Canvas canvas, CharSequence text, int start,
                         int end, float x, int top, int y, int bottom,
                         Paint paint) {
            int w = chipWidth(paint);
            RectF rect = new RectF(x + dp(1), y - dp(15),
                    x + dp(1) + w, y + dp(3));
            Paint bg = new Paint(Paint.ANTI_ALIAS_FLAG);
            bg.setColor(ink);
            bg.setStyle(Paint.Style.FILL);
            canvas.drawRoundRect(rect, dp(5), dp(5), bg);
            TextPaint p = chipPaint(paint);
            p.setTextAlign(Paint.Align.CENTER);
            Paint.FontMetricsInt fm = p.getFontMetricsInt();
            float baseline = rect.centerY() - (fm.ascent + fm.descent) / 2f;
            canvas.drawText(label, rect.centerX(), baseline, p);
        }
    }

    /** Answer text with valid [#id] markers turned into tappable
     *  chips (sequential numbers, as in the mock). A marker whose id
     *  is not among the turn's verified citations stays plain [id]
     *  text, as before. Display-only: the stored text and the
     *  citation parsing upstream stay untouched. */
    private SpannableStringBuilder citationText(String bodyText,
                                                  final List<Long> citeIds) {
        String cleaned = bodyText.replace("**", "")
                .replace("__", "")
                .replaceAll("(?m)^#{1,6} +", "");
        SpannableStringBuilder b = new SpannableStringBuilder();
        Matcher m = Pattern.compile("\\[#(\\d+)\\]").matcher(cleaned);
        int pos = 0;
        while (m.find()) {
            b.append(cleaned, pos, m.start());
            long id = -1;
            try {
                id = Long.parseLong(m.group(1));
            } catch (NumberFormatException e) {
                id = -1;
            }
            int idx = citeIds.indexOf(id);
            if (idx >= 0) {
                final long chunkId = id;
                String label = String.valueOf(idx + 1);
                int start = b.length();
                b.append(label);
                b.setSpan(new CiteChipSpan(label), start, b.length(),
                        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
                b.setSpan(new ClickableSpan() {
                    @Override
                    public void onClick(View w) {
                        showChunk(chunkId);
                    }

                    @Override
                    public void updateDrawState(TextPaint ds) {
                        // The chip span draws the range itself.
                    }
                }, start, b.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            } else {
                b.append("[").append(m.group(1)).append("]");
            }
            pos = m.end();
        }
        b.append(cleaned, pos, cleaned.length());
        return b;
    }

    /** Open one cited chunk: the source title as the heading, the
     *  chunk's own text as the body. A chunk id that no longer
     *  resolves (the source was re-chunked since) says so plainly. */
    private void showChunk(long chunkId) {
        String[][] rows = lines(Bridge.getChunk(chunkId));
        if (rows.length == 0 || rows[0].length < 2) {
            toast("出典の断片が見つかりませんでした");
            return;
        }
        String title = unesc(rows[0][0]);
        new android.app.AlertDialog.Builder(this)
                .setTitle(title.isEmpty() ? "出典" : title)
                .setMessage(unesc(rows[0][1]))
                .setPositiveButton("閉じる", null)
                .show();
    }

    /** Fallback chip row for a turn whose verified citations carry
     *  no inline marker (older records): one tappable chip per
     *  citation, numbered as the inline chips are. */
    private LinearLayout citationRow(final List<Long> citeIds) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        for (int i = 0; i < citeIds.size(); i++) {
            final long chunkId = citeIds.get(i);
            TextView chip = text(String.valueOf(i + 1), 12, ground, false, true);
            GradientDrawable bg = new GradientDrawable();
            bg.setColor(ink);
            bg.setCornerRadius(dp(5));
            chip.setBackground(bg);
            chip.setPadding(dp(8), dp(2), dp(8), dp(2));
            chip.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    showChunk(chunkId);
                }
            });
            LinearLayout.LayoutParams cp = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT);
            if (i > 0) cp.leftMargin = dp(6);
            chip.setLayoutParams(cp);
            row.addView(chip);
        }
        return row;
    }

    // ---------- nb2: chat ----------

    private void buildChat(LinearLayout body) {
        String[][] srcList = lines(Bridge.listSources(currentNb));
        int srcCount = srcList.length;

        LinearLayout card = column();
        card.setBackground(cardBg());
        card.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));
        card.addView(text(currentTitle, ssp(18), ink, true, true));
        TextView meta = text(srcCount + " 件のソース", ssp(12), sub, false, false);
        card.addView(meta);
        if (srcCount > 0) {
            StringBuilder names = new StringBuilder("資料：");
            for (int i = 0; i < srcList.length; i++) {
                if (i > 0) names.append("、");
                if (srcList[i].length > 1) names.append(unesc(srcList[i][1]));
            }
            TextView nl = text(names.toString(), 12, sub, false, false);
            LinearLayout.LayoutParams np = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT);
            np.topMargin = dp(4);
            nl.setLayoutParams(np);
            card.addView(nl);
        }
        body.addView(card);

        String[][] turns = lines(Bridge.listTurns(currentNb));
        LinearLayout thread = column();
        for (String[] turn : turns) {
            boolean user = turn[0].equals("user");
            String bodyText = unesc(turn.length > 1 ? turn[1] : "");
            String cites = turn.length > 2 ? turn[2] : "";
            if (user) {
                LinearLayout row = new LinearLayout(this);
                row.setGravity(Gravity.END);
                TextView bubble = text(bodyText, 15, ground, false, false);
                GradientDrawable bg = new GradientDrawable();
                bg.setColor(ink);
                bg.setCornerRadius(dp(14));
                bubble.setBackground(bg);
                bubble.setPadding(dp(12), dp(8), dp(12), dp(8));
                LinearLayout.LayoutParams bp = new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.WRAP_CONTENT,
                        LinearLayout.LayoutParams.WRAP_CONTENT);
                bp.leftMargin = dp(48);
                bubble.setLayoutParams(bp);
                row.addView(bubble);
                thread.addView(row, matchWrap(12));
            } else {
                // Citation markers [#id] render as small tappable
                // chips that open the cited chunk (see citationText).
                // The vessel has no Markdown renderer: the model's
                // emphasis markers would show literally, so they are
                // stripped there (display-only; the stored text and
                // the citation parsing upstream stay untouched).
                List<Long> citeIds = new ArrayList<>();
                if (!cites.isEmpty()) {
                    for (String part : cites.split(",")) {
                        try {
                            citeIds.add(Long.parseLong(part.trim()));
                        } catch (NumberFormatException e) {
                            // A malformed id simply gets no chip.
                        }
                    }
                }
                SpannableStringBuilder shown = citationText(bodyText, citeIds);
                TextView a = text("", 15, ink, false, false);
                a.setText(shown);
                if (shown.getSpans(0, shown.length(), CiteChipSpan.class).length > 0) {
                    a.setMovementMethod(LinkMovementMethod.getInstance());
                    a.setHighlightColor(Color.TRANSPARENT);
                }
                thread.addView(a, matchWrap(12));
                if (!citeIds.isEmpty()
                        && shown.getSpans(0, shown.length(), CiteChipSpan.class).length == 0) {
                    // No inline marker for these citations (older
                    // records): the chips go under the answer instead.
                    thread.addView(citationRow(citeIds), matchWrap(4));
                }
            }
        }
        if (turns.length == 0) {
            TextView hint = text(srcCount == 0
                    ? "まずソースのタブで資料を入れてください。"
                    : "下の欄から質問できます。", 14, sub, false, false);
            thread.addView(hint, matchWrap(14));
        }
        body.addView(thread);

        LinearLayout inputRow = new LinearLayout(this);
        inputRow.setOrientation(LinearLayout.HORIZONTAL);
        inputRow.setGravity(Gravity.CENTER_VERTICAL);
        final EditText input = new EditText(this);
        input.setHint("質問する");
        input.setTextColor(ink);
        input.setHintTextColor(sub);
        GradientDrawable fieldBg = new GradientDrawable();
        fieldBg.setColor(surface);
        fieldBg.setCornerRadius(dp(20));
        fieldBg.setStroke(1, hairline);
        input.setBackground(fieldBg);
        input.setPadding(dp(14), dp(8), dp(14), dp(8));
        input.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        inputRow.addView(input);
        Button send = pillBtn("↑");
        LinearLayout.LayoutParams sp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        sp.leftMargin = dp(8);
        send.setLayoutParams(sp);
        send.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                final String q = input.getText().toString().trim();
                if (q.isEmpty()) return;
                input.setText("");
                send.setEnabled(false);
                input.setHint("生成中…（端末内のモデルで考えています）");
                streamBuf.setLength(0);
                TextView bubble = text("", 15, ink, false, false);
                thread.addView(bubble, matchWrap(12));
                streamBubble = bubble;
                final long nb = currentNb;
                final String[] result = new String[1];
                runBg(new Runnable() {
                    @Override
                    public void run() {
                        result[0] = Bridge.ask(nb, q);
                    }
                }, new Runnable() {
                    @Override
                    public void run() {
                        streamBubble = null;
                        if (result[0] != null && result[0].startsWith("ERR:")) {
                            toast(errMsg(result[0]));
                        }
                        showNotebook(currentNb, currentTitle, 1);
                    }
                });
            }
        });
        inputRow.addView(send);
        body.addView(inputRow, matchWrap(14));
    }

    // ---------- nb3: sources ----------

    private void buildSources(LinearLayout body) {
        body.addView(text("ソース", 15, ink, false, true));
        String[][] srcs = lines(Bridge.listSources(currentNb));
        for (String[] s : srcs) {
            if (s.length < 2) continue;
            LinearLayout row = column();
            row.setBackground(cardBg());
            row.setPadding(dp(14), dp(10), dp(14), dp(10));
            row.addView(text(unesc(s[1]), 15, ink, false, false));
            body.addView(row, matchWrap(10));
        }
        if (srcs.length == 0) {
            TextView empty = text("まだ資料がありません。", 14, sub, false, false);
            body.addView(empty, matchWrap(10));
        }

        LinearLayout add = column();
        add.setBackground(cardBg());
        add.setPadding(dp(14), dp(12), dp(14), dp(12));
        final EditText srcTitle = new EditText(this);
        srcTitle.setHint("資料のタイトル");
        srcTitle.setTextColor(ink);
        srcTitle.setHintTextColor(sub);
        srcTitle.setBackgroundColor(Color.TRANSPARENT);
        add.addView(srcTitle);
        final EditText srcBody = new EditText(this);
        srcBody.setHint("本文（貼り付け）");
        srcBody.setTextColor(ink);
        srcBody.setHintTextColor(sub);
        srcBody.setBackgroundColor(Color.TRANSPARENT);
        srcBody.setMinLines(4);
        add.addView(srcBody);
        final Button addBtn = pillBtn("テキスト資料を追加");
        addBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                String t = srcTitle.getText().toString().trim();
                final String b = srcBody.getText().toString();
                if (b.trim().isEmpty()) return;
                if (t.isEmpty()) t = "テキスト資料";
                final String title = t;
                final long nb = currentNb;
                final long[] newId = new long[1];
                addBtn.setEnabled(false);
                addBtn.setText("索引作成中…");
                runBg(new Runnable() {
                    @Override
                    public void run() {
                        newId[0] = Bridge.addTextSource(nb, title, b);
                    }
                }, new Runnable() {
                    @Override
                    public void run() {
                        if (newId[0] < 0) toast("資料を追加できませんでした");
                        showNotebook(currentNb, currentTitle, 0);
                    }
                });
            }
        });
        add.addView(addBtn);
        body.addView(add, matchWrap(14));

        LinearLayout more = new LinearLayout(this);
        more.setOrientation(LinearLayout.HORIZONTAL);
        Button fileBtn = pillBtn("ファイルから追加");
        fileBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                android.content.Intent intent = new android.content.Intent(
                        android.content.Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(android.content.Intent.CATEGORY_OPENABLE);
                intent.setType("*/*");
                String[] mimes = {"text/plain", "text/markdown",
                        "application/epub+zip", "application/octet-stream"};
                intent.putExtra(android.content.Intent.EXTRA_MIME_TYPES, mimes);
                try {
                    startActivityForResult(intent, REQ_PICK_FILE);
                } catch (Throwable t) {
                    toast("ファイル選択を開けませんでした");
                }
            }
        });
        more.addView(fileBtn);
        Button webBtn = pillBtn("Webで探す");
        LinearLayout.LayoutParams wp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        wp.leftMargin = dp(8);
        webBtn.setLayoutParams(wp);
        webBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showWebSearch();
            }
        });
        more.addView(webBtn);
        body.addView(more, matchWrap(12));
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode,
            android.content.Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQ_PICK_FILE || resultCode != RESULT_OK
                || data == null || data.getData() == null || currentNb < 0) {
            return;
        }
        final android.net.Uri uri = data.getData();
        final long nb = currentNb;
        toast("ファイルを読み込んでいます…");
        runBg(new Runnable() {
            @Override
            public void run() {
                try {
                    String name = "ファイル資料";
                    android.database.Cursor c = getContentResolver().query(
                            uri, null, null, null, null);
                    if (c != null) {
                        if (c.moveToFirst()) {
                            int ix = c.getColumnIndex(
                                    android.provider.OpenableColumns.DISPLAY_NAME);
                            if (ix >= 0) name = c.getString(ix);
                        }
                        c.close();
                    }
                    java.io.InputStream in = getContentResolver().openInputStream(uri);
                    java.io.ByteArrayOutputStream bos = new java.io.ByteArrayOutputStream();
                    byte[] buf = new byte[65536];
                    int r;
                    while ((r = in.read(buf)) > 0) bos.write(buf, 0, r);
                    in.close();
                    String lower = name.toLowerCase(Locale.JAPAN);
                    String kind = lower.endsWith(".epub") ? "epub" : "txt";
                    String title = name.replaceAll("\\.[A-Za-z0-9]+$", "");
                    long id = Bridge.addFileSource(nb, title, kind, bos.toByteArray());
                    final boolean ok = id >= 0;
                    runOnUiThread(new Runnable() {
                        @Override
                        public void run() {
                            toast(ok ? "資料を追加しました" : "読み込めませんでした（対応形式：EPUB / TXT / MD）");
                        }
                    });
                } catch (Throwable t) {
                    runOnUiThread(new Runnable() {
                        @Override
                        public void run() {
                            toast("読み込みに失敗しました");
                        }
                    });
                }
            }
        }, new Runnable() {
            @Override
            public void run() {
                if (currentNb == nb) showNotebook(nb, currentTitle, 0);
            }
        });
    }

    /** Web search screen: query the web seat, list hits, and add a
     *  hit's page as a source of the current notebook. */
    private void showWebSearch() {
        clearScreen();
        LinearLayout head = new LinearLayout(this);
        head.setOrientation(LinearLayout.HORIZONTAL);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView back = text("←", ssp(20), ink, false, false);
        back.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showNotebook(currentNb, currentTitle, 0);
            }
        });
        head.addView(back);
        TextView t = text("Webで探す", ssp(19), ink, false, true);
        LinearLayout.LayoutParams tp = new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f);
        tp.leftMargin = dp(10);
        t.setLayoutParams(tp);
        head.addView(t);
        screen.addView(head);

        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER_VERTICAL);
        final EditText q = new EditText(this);
        q.setHint("検索のことば");
        q.setTextColor(ink);
        q.setHintTextColor(sub);
        GradientDrawable fieldBg = new GradientDrawable();
        fieldBg.setColor(surface);
        fieldBg.setCornerRadius(dp(20));
        fieldBg.setStroke(1, hairline);
        q.setBackground(fieldBg);
        q.setPadding(dp(14), dp(8), dp(14), dp(8));
        q.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        row.addView(q);
        final Button go = pillBtn("検索");
        LinearLayout.LayoutParams gp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        gp.leftMargin = dp(8);
        go.setLayoutParams(gp);
        row.addView(go);
        screen.addView(row, matchWrap(14));

        final LinearLayout results = column();
        screen.addView(results, matchWrap(10));
        final long nb = currentNb;
        final String nbTitle = currentTitle;
        go.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                final String query = q.getText().toString().trim();
                if (query.isEmpty()) return;
                go.setEnabled(false);
                results.removeAllViews();
                results.addView(text("検索中…", 14, sub, false, false));
                final String[] raw = new String[1];
                runBg(new Runnable() {
                    @Override
                    public void run() {
                        raw[0] = Bridge.webSearch(query);
                    }
                }, new Runnable() {
                    @Override
                    public void run() {
                        go.setEnabled(true);
                        results.removeAllViews();
                        if (raw[0] != null && raw[0].startsWith("ERR:")) {
                            results.addView(text(errMsg(raw[0]),
                                    14, sub, false, false));
                            return;
                        }
                        String[][] hits = lines(raw[0]);
                        if (hits.length == 0) {
                            results.addView(text("見つかりませんでした。",
                                    14, sub, false, false));
                            return;
                        }
                        for (final String[] hit : hits) {
                            if (hit.length < 2) continue;
                            LinearLayout card = column();
                            card.setBackground(cardBg());
                            card.setPadding(dp(14), dp(10), dp(14), dp(10));
                            card.addView(text(unesc(hit[0]), 15, ink, false, false));
                            card.addView(text(unesc(hit[1]), 12, sub, false, false));
                            if (hit.length > 2 && !hit[2].isEmpty()) {
                                card.addView(text(unesc(hit[2]), 12, sub, false, false));
                            }
                            card.addView(text("タップで資料に追加", 12, sub, false, false));
                            card.setOnClickListener(new View.OnClickListener() {
                                @Override
                                public void onClick(View v) {
                                    toast("ページを取り込んでいます…");
                                    final String[] res = new String[1];
                                    runBg(new Runnable() {
                                        @Override
                                        public void run() {
                                            res[0] = Bridge.addWebSource(nb,
                                                    unesc(hit[0]), unesc(hit[1]));
                                        }
                                    }, new Runnable() {
                                        @Override
                                        public void run() {
                                            if (res[0] != null && res[0].startsWith("OK")) {
                                                toast("資料を追加しました");
                                                showNotebook(nb, nbTitle, 0);
                                            } else {
                                                toast("取り込めませんでした");
                                            }
                                        }
                                    });
                                }
                            });
                            results.addView(card, matchWrap(10));
                        }
                    }
                });
            }
        });
    }

    // ---------- nb4: Studio (audio overview only; generation is stage 7) ----------

private void buildStudio(LinearLayout body) {
        body.addView(text("Studio", 15, ink, false, true));

        LinearLayout card = column();
        card.setBackground(cardBg());
        card.setPadding(dp(14), dp(12), dp(14), dp(12));
        body.addView(card, matchWrap(0));

        card.addView(text("音声概要", 13, sub, false, false));
        card.addView(text("ソースを、二人の対話で聴く", 18, ink, true, true));
        card.addView(text("選択中のソースから、対話形式の音声を生成します。台本は API のモデルが書き、合成は端末の中で行います。", 13, sub, false, false));

        String aName = "", bName = "";
        try {
            org.json.JSONArray catalog = new org.json.JSONArray(Bridge.getVoiceCatalog());
            for (int i = 0; i < catalog.length(); i++) {
                org.json.JSONObject v = catalog.getJSONObject(i);
                if (v.optBoolean("isA")) aName = v.optString("name", v.optString("key", ""));
                if (v.optBoolean("isB")) bName = v.optString("name", v.optString("key", ""));
            }
        } catch (Exception ignored) {}
        if (aName.isEmpty()) aName = "A";
        if (bName.isEmpty()) bName = "B";

        LinearLayout voiceRow = new LinearLayout(this);
        voiceRow.setOrientation(LinearLayout.HORIZONTAL);
        voiceRow.setGravity(android.view.Gravity.CENTER_VERTICAL);
        voiceRow.addView(text("声", 12, sub, false, false));
        voiceRow.addView(text(aName + "／" + bName, 13, ink, false, true), new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        TextView chev = text("›", 16, sub, false, false);
        voiceRow.addView(chev);
        voiceRow.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) { showSettings(); }
        });
        card.addView(voiceRow, matchWrap(8));

        final EditText focusEt = new EditText(this);
        focusEt.setHint("焦点（任意）：例「水門の整備史を中心に」");
        focusEt.setTextColor(ink);
        focusEt.setBackgroundColor(android.graphics.Color.TRANSPARENT);
        focusEt.setTextSize(android.util.TypedValue.COMPLEX_UNIT_SP, 13);
        focusEt.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE);
        focusEt.setGravity(android.view.Gravity.TOP | android.view.Gravity.START);
        focusEt.setPadding(dp(12), dp(8), dp(12), dp(8));
        focusEt.setLayoutParams(new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(56)));

        final Button genBtn = pillBtn("生成する");
        LinearLayout.LayoutParams btnLp = new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT);
        btnLp.topMargin = dp(8);
        card.addView(genBtn, btnLp);

        final TextView statusTv = text("", 12, sub, false, false);
        statusTv.setVisibility(View.GONE);
        card.addView(statusTv, matchWrap(4));

        genBtn.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                genBtn.setEnabled(false);
                statusTv.setText("生成中…");
                statusTv.setVisibility(View.VISIBLE);
                final String focus = focusEt.getText().toString().trim();
                final String dictDir = new java.io.File(getFilesDir(), "open_jtalk_dic").getAbsolutePath();
                final String ortLib = getApplicationInfo().nativeLibraryDir + "/libvoicevox_onnxruntime.so";
                final long at = System.currentTimeMillis();
                runBg(new Runnable() {
                    @Override public void run() {
                        ensureVoiceDict();
                        final String res = Bridge.generateVoiceSummary(currentNb, focus, dictDir, ortLib, at);
                        runOnUiThread(new Runnable() {
                            @Override public void run() {
                                genBtn.setEnabled(true);
                                statusTv.setVisibility(View.GONE);
                                if (res.startsWith("ERR:")) {
                                    toast(errMsg(res));
                                } else {
                                    toast("音声概要を生成しました");
                                }
                                showNotebook(currentNb, currentTitle, 2);
                            }
                        });
                    }
                }, null);
            }
        });

        LinearLayout focusCard = column();
        focusCard.setBackground(cardBg());
        focusCard.setPadding(dp(14), dp(6), dp(14), dp(6));
        focusCard.addView(focusEt);
        body.addView(focusCard, matchWrap(10));

        body.addView(text("生成済み", 14, ink, false, true), matchWrap(16));

        String listJson = Bridge.listVoiceSummaries(currentNb);
        try {
            org.json.JSONArray arr = new org.json.JSONArray(listJson);
            if (arr.length() == 0) {
                body.addView(text("まだ音声概要はありません。", 13, sub, false, false), matchWrap(8));
            } else {
                for (int i = 0; i < arr.length(); i++) {
                    final org.json.JSONObject rec = arr.getJSONObject(i);
                    LinearLayout row = new LinearLayout(this);
                    row.setBackground(cardBg());
                    row.setOrientation(LinearLayout.HORIZONTAL);
                    row.setGravity(android.view.Gravity.CENTER_VERTICAL);
                    row.setPadding(dp(12), dp(10), dp(12), dp(10));

                    TextView playTv = text("▶", 16, ink, false, false);
                    LinearLayout.LayoutParams playLp = new LinearLayout.LayoutParams(dp(36), dp(36));
                    playLp.rightMargin = dp(10);
                    row.addView(playTv, playLp);

                    LinearLayout infoCol = column();
                    infoCol.addView(text(rec.optString("title", ""), 15, ink, false, true));
                    String subText = "音声概要 ・ " + fmtDur(rec.optDouble("durationSec", 0)) + " ・ " + dateOf(rec.optLong("createdAt", 0));
                    infoCol.addView(text(subText, 12, sub, false, false));
                    LinearLayout.LayoutParams infoLp = new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1);
                    row.addView(infoCol, infoLp);

                    TextView menuTv = text("⋮", 18, sub, false, false);
                    menuTv.setPadding(dp(8), 0, 0, 0);
                    menuTv.setOnClickListener(new View.OnClickListener() {
                        @Override public void onClick(View v) {
                            new android.app.AlertDialog.Builder(MainActivity.this)
                                .setItems(new CharSequence[]{"削除"}, new android.content.DialogInterface.OnClickListener() {
                                    @Override public void onClick(android.content.DialogInterface dialog, int which) {
                                        runBg(new Runnable() {
                                            @Override public void run() {
                                                Bridge.deleteVoiceSummary(rec.optLong("id", 0));
                                                runOnUiThread(new Runnable() {
                                                    @Override public void run() {
                                                        showNotebook(currentNb, currentTitle, 2);
                                                    }
                                                });
                                            }
                                        }, null);
                                    }
                                })
                                .show();
                        }
                    });
                    row.addView(menuTv);

                    row.setOnClickListener(new View.OnClickListener() {
                        @Override public void onClick(View v) { showPlayer(rec); }
                    });
                    body.addView(row, matchWrap(8));
                }
            }
        } catch (Exception e) {
            body.addView(text("読み込みエラー", 13, sub, false, false), matchWrap(8));
        }
    }

    private String fmtDur(double sec) {
        long total = Math.round(sec);
        long m = total / 60;
        long s = total % 60;
        return String.format(Locale.US, "%d:%02d", m, s);
    }

    private void showPlayer(final org.json.JSONObject rec) {
        stopPlayer();
        clearScreen();

        LinearLayout header = new LinearLayout(this);
        header.setOrientation(LinearLayout.HORIZONTAL);
        header.setGravity(android.view.Gravity.CENTER_VERTICAL);
        header.setPadding(dp(12), dp(8), dp(12), dp(8));
        TextView backTv = text("←", ssp(20), ink, false, false);
        backTv.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                stopPlayer();
                showNotebook(currentNb, currentTitle, 2);
            }
        });
        header.addView(backTv);
        header.addView(text("音声概要", ssp(19), ink, false, true), new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        screen.addView(header);


        TextView playerTitle = text(rec.optString("title", ""), 18, ink, true, true);
        playerTitle.setGravity(android.view.Gravity.CENTER);
        screen.addView(playerTitle, matchWrap(24));
        TextView playerMeta = text("音声概要 ・ " + dateOf(rec.optLong("createdAt", 0)), 12, sub, false, false);
        playerMeta.setGravity(android.view.Gravity.CENTER);
        screen.addView(playerMeta, matchWrap(6));

        String vaKey = rec.optString("voiceA", "");
        String vbKey = rec.optString("voiceB", "");
        String vaName = vaKey, vbName = vbKey;
        try {
            org.json.JSONArray catalog = new org.json.JSONArray(Bridge.getVoiceCatalog());
            for (int i = 0; i < catalog.length(); i++) {
                org.json.JSONObject v = catalog.getJSONObject(i);
                String k = v.optString("key", "");
                if (k.equals(vaKey)) vaName = v.optString("name", k);
                if (k.equals(vbKey)) vbName = v.optString("name", k);
            }
        } catch (Exception ignored) {}

        LinearLayout hosts = new LinearLayout(this);
        hosts.setOrientation(LinearLayout.HORIZONTAL);
        hosts.setGravity(android.view.Gravity.CENTER);
        hosts.addView(text("A " + vaName, 12, sub, false, false));
        TextView sep = text("  ・  ", 12, sub, false, false);
        hosts.addView(sep);
        hosts.addView(text("B " + vbName, 12, sub, false, false));
        screen.addView(hosts, matchWrap(16));

        final android.widget.SeekBar seek = new android.widget.SeekBar(this);
        int durMs = (int) (rec.optDouble("durationSec", 0) * 1000);
        seek.setMax(durMs);
        LinearLayout.LayoutParams seekLp = new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT);
        seekLp.leftMargin = dp(16);
        seekLp.rightMargin = dp(16);
        seekLp.topMargin = dp(16);
        screen.addView(seek, seekLp);

        LinearLayout timeRow = new LinearLayout(this);
        timeRow.setOrientation(LinearLayout.HORIZONTAL);
        final TextView curTv = text("0:00", 12, sub, false, false);
        final TextView totTv = text(fmtDur(rec.optDouble("durationSec", 0)), 12, sub, false, false);
        timeRow.addView(curTv);
        timeRow.addView(new View(this), new LinearLayout.LayoutParams(0, 0, 1));
        timeRow.addView(totTv);
        timeRow.setPadding(dp(16), dp(4), dp(16), 0);
        screen.addView(timeRow, matchWrap(0));

        LinearLayout ctrlRow = new LinearLayout(this);
        ctrlRow.setOrientation(LinearLayout.HORIZONTAL);
        ctrlRow.setGravity(android.view.Gravity.CENTER);
        ctrlRow.setPadding(0, dp(12), 0, dp(16));

        final TextView speedTv = text("1.0x", 13, sub, false, false);
        final float[] speeds = {0.75f, 1.0f, 1.25f, 1.5f};
        final int[] speedIdx = {1};
        speedTv.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                speedIdx[0] = (speedIdx[0] + 1) % speeds.length;
                float sp = speeds[speedIdx[0]];
                speedTv.setText(new String[]{"0.75x", "1.0x", "1.25x", "1.5x"}[speedIdx[0]]);
                if (player != null) {
                    android.media.PlaybackParams pp = player.getPlaybackParams();
                    pp.setSpeed(sp);
                    player.setPlaybackParams(pp);
                }
            }
        });
        ctrlRow.addView(speedTv);
        ctrlRow.addView(new View(this), new LinearLayout.LayoutParams(dp(24), 0));

        TextView rwTv = text("⏮", 15, ink, false, false);
        rwTv.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                if (player != null) {
                    try {
                        int pos = player.getCurrentPosition() - 10000;
                        if (pos < 0) pos = 0;
                        player.seekTo(pos);
                    } catch (Exception ignored) {}
                }
            }
        });
        ctrlRow.addView(rwTv);
        ctrlRow.addView(new View(this), new LinearLayout.LayoutParams(dp(16), 0));

        final TextView playPauseTv = text("⏸", 30, ink, false, false);
        playPauseTv.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                if (player != null) {
                    if (player.isPlaying()) {
                        player.pause();
                        playPauseTv.setText("▶");
                    } else {
                        try {
                            player.start();
                            playPauseTv.setText("⏸");
                        } catch (Exception ignored) {}
                    }
                }
            }
        });
        ctrlRow.addView(playPauseTv);
        ctrlRow.addView(new View(this), new LinearLayout.LayoutParams(dp(16), 0));

        TextView ffTv = text("⏭", 15, ink, false, false);
        ffTv.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                if (player != null) {
                    try {
                        int pos = player.getCurrentPosition() + 10000;
                        if (pos > player.getDuration()) pos = player.getDuration();
                        player.seekTo(pos);
                    } catch (Exception ignored) {}
                }
            }
        });
        ctrlRow.addView(ffTv);

        screen.addView(ctrlRow, matchWrap(0));

        final String wavFile = rec.optString("wavFile", "");
        java.io.File f = new java.io.File(new java.io.File(getFilesDir(), "audio"), wavFile);
        if (!f.exists()) {
            toast("音声ファイルが見つかりません");
            return;
        }

        player = new android.media.MediaPlayer();
        playerHandler = new android.os.Handler(android.os.Looper.getMainLooper());
        try {
            player.setDataSource(f.getAbsolutePath());
            player.setOnCompletionListener(new android.media.MediaPlayer.OnCompletionListener() {
                @Override public void onCompletion(android.media.MediaPlayer mp) {
                    if (playerHandler != null && playerTick != null) {
                        playerHandler.removeCallbacks(playerTick);
                    }
                    playPauseTv.setText("▶");
                    seek.setProgress(0);
                    curTv.setText("0:00");
                }
            });
            seek.setOnSeekBarChangeListener(new android.widget.SeekBar.OnSeekBarChangeListener() {
                boolean user = false;
                @Override public void onStartTrackingTouch(android.widget.SeekBar sb) { user = true; }
                @Override public void onStopTrackingTouch(android.widget.SeekBar sb) {
                    user = false;
                    if (player != null) {
                        try { player.seekTo(sb.getProgress()); } catch (Exception ignored) {}
                    }
                }
                @Override public void onProgressChanged(android.widget.SeekBar sb, int prog, boolean fromUser) {
                    if (fromUser && player != null) {
                        try { player.seekTo(prog); } catch (Exception ignored) {}
                    }
                }
            });

            playerTick = new Runnable() {
                @Override public void run() {
                    if (player != null) {
                        int pos = player.getCurrentPosition();
                        seek.setProgress(pos);
                        curTv.setText(fmtDur(pos / 1000.0));
                        playerHandler.postDelayed(this, 500);
                    }
                }
            };
            player.setOnPreparedListener(new android.media.MediaPlayer.OnPreparedListener() {
                @Override public void onPrepared(android.media.MediaPlayer mp) {
                    mp.start();
                    playPauseTv.setText("⏸");
                    playerHandler.post(playerTick);
                }
            });
            player.prepareAsync();
        } catch (Exception e) {
            toast("再生準備エラー");
            stopPlayer();
        }
    }

    private void stopPlayer() {
        if (playerHandler != null && playerTick != null) {
            playerHandler.removeCallbacks(playerTick);
        }
        if (player != null) {
            try {
                if (player.isPlaying()) player.stop();
            } catch (Exception ignored) {}
            player.release();
            player = null;
        }
    }

    private void ensureVoiceDict() {
        java.io.File dictDir = new java.io.File(getFilesDir(), "open_jtalk_dic");
        java.io.File sysDic = new java.io.File(dictDir, "sys.dic");
        if (sysDic.exists()) return;
        if (!dictDir.mkdirs()) return;
        try {
            String[] assets = getAssets().list("open_jtalk_dic");
            if (assets == null) return;
            for (String name : assets) {
                java.io.InputStream in = getAssets().open("open_jtalk_dic/" + name);
                java.io.FileOutputStream out = new java.io.FileOutputStream(new java.io.File(dictDir, name));
                byte[] buf = new byte[8192];
                int len;
                while ((len = in.read(buf)) > 0) out.write(buf, 0, len);
                in.close();
                out.close();
            }
        } catch (Exception ignored) {}
    }

private View voiceSlotRow(final String letter, final org.json.JSONObject entry,
                              final boolean slotA, final org.json.JSONArray catalog) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(android.view.Gravity.CENTER_VERTICAL);
        LinearLayout left = column();
        String name = entry != null ? entry.optString("name", "未設定") : "未設定";
        boolean installed = entry != null && entry.optBoolean("installed", false);
        left.addView(text(name, 15, ink, false, true));
        left.addView(text("ホスト " + letter + " ・ " + (installed ? "導入済み" : "未導入"), 12, sub, false, false));
        row.addView(left, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        row.addView(text("›", 16, sub, false, false));
        row.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) { pickVoiceForSlot(slotA, catalog); }
        });
        return row;
    }

    private void buildVoiceSettings() {
        String json = Bridge.getVoiceCatalog();
        if (json == null) return;
        org.json.JSONArray catalog;
        try {
            catalog = new org.json.JSONArray(json);
        } catch (org.json.JSONException e) {
            return;
        }

        screen.addView(text("音声概要", 13, sub, false, true), matchWrap(16));

        LinearLayout scriptCard = column();
        scriptCard.setBackground(cardBg());
        scriptCard.setPadding(dp(14), dp(12), dp(14), dp(12));
        scriptCard.addView(text("台本", 15, ink, false, true), matchWrap(0));
        scriptCard.addView(text("生成には API 設定が必要です。台本は API のモデルが書きます", 12, sub, false, false), matchWrap(4));
        screen.addView(scriptCard, matchWrap(8));

        screen.addView(text("対話の声", 13, sub, false, true), matchWrap(16));

        String currentA = null;
        String currentB = null;
        org.json.JSONObject entryA = null;
        org.json.JSONObject entryB = null;

        for (int i = 0; i < catalog.length(); i++) {
            try {
                org.json.JSONObject v = catalog.getJSONObject(i);
                if (v.optBoolean("isA", false)) {
                    currentA = v.optString("key", "");
                    entryA = v;
                }
                if (v.optBoolean("isB", false)) {
                    currentB = v.optString("key", "");
                    entryB = v;
                }
            } catch (org.json.JSONException ignored) {}
        }

        final String finalCurrentA = currentA;
        final String finalCurrentB = currentB;
        final org.json.JSONObject finalEntryA = entryA;
        final org.json.JSONObject finalEntryB = entryB;

        final org.json.JSONArray catalogF = catalog;
        LinearLayout slotCard = column();
        slotCard.setBackground(cardBg());
        slotCard.setPadding(dp(14), dp(12), dp(14), dp(12));
        slotCard.addView(voiceSlotRow("A", finalEntryA, true, catalogF));
        LinearLayout divider = new LinearLayout(this);
        divider.setLayoutParams(new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(1)));
        divider.setBackgroundColor(hairline);
        LinearLayout.LayoutParams divLp = new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(1));
        divLp.topMargin = dp(10);
        divLp.bottomMargin = dp(10);
        divider.setLayoutParams(divLp);
        slotCard.addView(divider);
        slotCard.addView(voiceSlotRow("B", finalEntryB, false, catalogF));
        screen.addView(slotCard, matchWrap(8));

        screen.addView(text("候補", 13, sub, false, true), matchWrap(16));

        LinearLayout candidatesCard = column();
        candidatesCard.setBackground(cardBg());
        candidatesCard.setPadding(dp(14), dp(12), dp(14), dp(12));

        for (int i = 0; i < catalog.length(); i++) {
            try {
                final org.json.JSONObject v = catalog.getJSONObject(i);
                final String key = v.optString("key", "");
                final String name = v.optString("name", "");
                final String credit = v.optString("credit", "");
                final boolean installed = v.optBoolean("installed", false);
                final boolean consented = v.optBoolean("consented", false);
                final boolean downloading = voiceDownloading.contains(key);

                LinearLayout row = new LinearLayout(this);
                row.setOrientation(LinearLayout.HORIZONTAL);
                row.setGravity(android.view.Gravity.CENTER_VERTICAL);
                row.setLayoutParams(matchWrap(8));

                LinearLayout leftCol = column();
                leftCol.addView(text(name, 15, ink, false, true), matchWrap(0));
                leftCol.addView(text("クレジット表記 \"" + credit + "\"", 12, sub, false, false), matchWrap(0));
                row.addView(leftCol, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

                TextView rightTv;
                if (installed) {
                    rightTv = text("導入済み ✓", 12, sub, false, false);
                    row.addView(rightTv, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT));
                } else if (downloading) {
                    rightTv = text("ダウンロード中…", 12, sub, false, false);
                    progressSink = rightTv;
                    progressSinkKey = key;
                    row.addView(rightTv, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT));
                } else {
                    rightTv = text("⬇", 18, ink, false, false);
                    rightTv.setLayoutParams(new LinearLayout.LayoutParams(dp(40), LinearLayout.LayoutParams.WRAP_CONTENT));
                    rightTv.setGravity(android.view.Gravity.CENTER);
                    rightTv.setOnClickListener(new android.view.View.OnClickListener() {
                        @Override public void onClick(android.view.View view) {
                            if (!consented) {
                                showVoiceConsent(v);
                            } else {
                                startVoiceDownload(v);
                            }
                        }
                    });
                    row.addView(rightTv);
                }
                candidatesCard.addView(row);
            } catch (org.json.JSONException ignored) {}
        }
        screen.addView(candidatesCard, matchWrap(8));

        screen.addView(text("声は VOICEVOX 公式リポジトリから、この端末へ直接ダウンロードされます。ダウンロードの前に、各声の利用規約への同意が必要です。", 11.5f, sub, false, false), matchWrap(8));
        String creditA = finalEntryA != null ? finalEntryA.optString("credit", "") : "";
        String creditB = finalEntryB != null ? finalEntryB.optString("credit", "") : "";
        screen.addView(text("音声合成 VOICEVOX ／ " + creditA + " ・ " + creditB, 11.5f, sub, false, false), matchWrap(8));
    }

    private void startVoiceDownload(final org.json.JSONObject v) {
        final String key;
        try { key = v.getString("key"); } catch (org.json.JSONException e) { return; }
        voiceDownloading.add(key);
        showSettings();
        final String[] res = new String[1];
        runBg(new Runnable() {
            @Override public void run() {
                res[0] = Bridge.downloadVoice(key);
            }
        }, new Runnable() {
            @Override public void run() {
                voiceDownloading.remove(key);
                if (res[0] != null && res[0].startsWith("ERR:")) {
                    toast(res[0]);
                } else {
                    toast("ダウンロードしました");
                }
                showSettings();
            }
        });
    }

    private void showVoiceConsent(final org.json.JSONObject v) {
        clearScreen();
        final String key;
        final String name;
        final String credit;
        final String termsUrl;
        try {
            key = v.getString("key");
            name = v.getString("name");
            credit = v.getString("credit");
            termsUrl = v.getString("termsUrl");
        } catch (org.json.JSONException e) {
            showSettings();
            return;
        }

        LinearLayout header = new LinearLayout(this);
        header.setOrientation(LinearLayout.HORIZONTAL);
        header.setGravity(android.view.Gravity.CENTER_VERTICAL);
        header.setLayoutParams(matchWrap(0));
        TextView back = text("←", ssp(20), ink, false, false);
        back.setLayoutParams(new LinearLayout.LayoutParams(dp(48), dp(48)));
        back.setGravity(android.view.Gravity.CENTER);
        back.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) { showSettings(); }
        });
        header.addView(back);
        header.addView(text("利用規約", ssp(19), ink, false, true), new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        screen.addView(header, matchWrap(0));

        screen.addView(text(name, 17, ink, true, true), matchWrap(4));
        screen.addView(text("音声ライブラリ利用規約", 12, sub, false, false), matchWrap(12));

        LinearLayout pointsCard = column();
        pointsCard.setBackground(cardBg());
        pointsCard.setPadding(dp(14), dp(12), dp(14), dp(12));
        pointsCard.addView(text("・商用・非商用を問わず利用できます", 13, sub, false, false), matchWrap(4));
        pointsCard.addView(text("・音声を使うときは『" + credit + "』のクレジット表記が必要です", 13, sub, false, false), matchWrap(4));
        pointsCard.addView(text("・禁止：政治・宗教活動、情報商材、虚偽情報の作成、風俗営業、反社会的勢力による利用", 13, sub, false, false), matchWrap(4));
        pointsCard.addView(text("・声そのもの（音声モデル）の再配布はできません", 13, sub, false, false), matchWrap(4));
        screen.addView(pointsCard, matchWrap(8));

        TextView termsLink = text("規約全文を読む ›", 13, ink, false, false);
        termsLink.setLayoutParams(matchWrap(12));
        termsLink.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) {
                try {
                    android.content.Intent intent = new android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(termsUrl));
                    startActivity(intent);
                } catch (Exception ignored) {}
            }
        });
        screen.addView(termsLink);

        screen.addView(text("同意はこの声について記録され、次回からこの画面は表示されません。", 11.5f, sub, false, false), matchWrap(16));

        LinearLayout bottomRow = new LinearLayout(this);
        bottomRow.setOrientation(LinearLayout.HORIZONTAL);
        bottomRow.setGravity(android.view.Gravity.CENTER_VERTICAL);
        bottomRow.setLayoutParams(matchWrap(0));
        TextView cancel = text("キャンセル", 14, sub, false, false);
        cancel.setLayoutParams(new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        cancel.setGravity(android.view.Gravity.CENTER);
        cancel.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) { showSettings(); }
        });
        bottomRow.addView(cancel);
        Button agreeBtn = pillBtn("同意してダウンロード");
        agreeBtn.setLayoutParams(new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        agreeBtn.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) {
                final String[] msg = new String[1];
                runBg(new Runnable() {
                    @Override public void run() {
                        String consentRes = Bridge.recordVoiceConsent(key);
                        if (consentRes != null && consentRes.startsWith("ERR:")) {
                            msg[0] = errMsg(consentRes);
                        } else {
                            String dlRes = Bridge.downloadVoice(key);
                            msg[0] = (dlRes != null && dlRes.startsWith("ERR:")) ? errMsg(dlRes) : "ダウンロードしました";
                        }
                    }
                }, new Runnable() {
                    @Override public void run() {
                        toast(msg[0]);
                        showSettings();
                    }
                });
            }
        });
        bottomRow.addView(agreeBtn);
        screen.addView(bottomRow, matchWrap(16));
    }

    private void pickVoiceForSlot(final boolean slotA, final org.json.JSONArray catalog) {
        final String[] names = new String[catalog.length()];
        final String[] keys = new String[catalog.length()];
        for (int i = 0; i < catalog.length(); i++) {
            try {
                org.json.JSONObject v = catalog.getJSONObject(i);
                names[i] = v.optString("name", "");
                keys[i] = v.optString("key", "");
            } catch (org.json.JSONException e) {
                names[i] = "";
                keys[i] = "";
            }
        }

        new android.app.AlertDialog.Builder(this)
            .setTitle(slotA ? "ホスト A の声を選択" : "ホスト B の声を選択")
            .setItems(names, new android.content.DialogInterface.OnClickListener() {
                @Override public void onClick(android.content.DialogInterface dialog, int which) {
                    String selectedKey = keys[which];
                    String newA, newB;
                    if (slotA) {
                        newA = selectedKey;
                        newB = (selectedKey.equals(getCurrentVoiceKey(false))) ? getCurrentVoiceKey(true) : getCurrentVoiceKey(false);
                    } else {
                        newB = selectedKey;
                        newA = (selectedKey.equals(getCurrentVoiceKey(true))) ? getCurrentVoiceKey(false) : getCurrentVoiceKey(true);
                    }
                    final String finalNewA = newA;
                    final String finalNewB = newB;
                    runBg(new Runnable() {
                        @Override public void run() {
                            Bridge.setVoicePair(finalNewA, finalNewB);
                        }
                    }, new Runnable() {
                        @Override public void run() {
                            showSettings();
                        }
                    });
                }
            })
            .show();
    }

    private String getCurrentVoiceKey(boolean isA) {
        String json = Bridge.getVoiceCatalog();
        if (json == null) return "";
        try {
            org.json.JSONArray catalog = new org.json.JSONArray(json);
            for (int i = 0; i < catalog.length(); i++) {
                org.json.JSONObject v = catalog.getJSONObject(i);
                if (isA && v.optBoolean("isA", false)) return v.optString("key", "");
                if (!isA && v.optBoolean("isB", false)) return v.optString("key", "");
            }
        } catch (org.json.JSONException ignored) {}
        return "";
    }

private void seedFixtureIfPresent() {
        runBg(new Runnable() {
            @Override
            public void run() {
                try {
                    // 1. Check GGUF asset existence
                    java.io.InputStream testIn = null;
                    try {
                        testIn = getAssets().open("fixture/tinyswallow-1.5b-instruct-q5_k_m.gguf");
                    } catch (java.io.IOException e) {
                        // production build: asset absent
                        return;
                    } finally {
                        if (testIn != null) try { testIn.close(); } catch (java.io.IOException ignored) {}
                    }

                    // 2. Copy GGUF to filesDir/models if not already present
                    java.io.File modelsDir = new java.io.File(getFilesDir(), "models");
                    if (!modelsDir.exists()) modelsDir.mkdirs();
                    java.io.File ggufOut = new java.io.File(modelsDir, "tinyswallow-1.5b-instruct-q5_k_m.gguf");
                    if (!ggufOut.exists()) {
                        // Copy atomically: write to a .part sibling and
                        // rename only on success, so a failed copy never
                        // leaves an incomplete file that exists() would
                        // mistake for a completed one on the next launch.
                        java.io.File ggufPart = new java.io.File(modelsDir, "tinyswallow-1.5b-instruct-q5_k_m.gguf.part");
                        java.io.InputStream in = null;
                        java.io.FileOutputStream fos = null;
                        boolean copied = false;
                        try {
                            in = getAssets().open("fixture/tinyswallow-1.5b-instruct-q5_k_m.gguf");
                            fos = new java.io.FileOutputStream(ggufPart);
                            byte[] buf = new byte[65536];
                            int r;
                            while ((r = in.read(buf)) > 0) {
                                fos.write(buf, 0, r);
                            }
                            copied = true;
                        } catch (java.io.IOException e) {
                            // copy failed: the .part file is removed below
                        } finally {
                            if (in != null) try { in.close(); } catch (java.io.IOException ignored) {}
                            if (fos != null) try { fos.close(); } catch (java.io.IOException ignored) {}
                        }
                        if (copied) {
                            if (!ggufPart.renameTo(ggufOut)) {
                                ggufPart.delete();
                            }
                        } else {
                            ggufPart.delete();
                        }
                    }

                    // 3. EPUB seeding with marker
                    java.io.File marker = new java.io.File(getFilesDir(), "fixture_seeded");
                    if (marker.exists()) {
                        return;
                    }

                    // Check EPUB asset
                    java.io.InputStream epubIn = null;
                    try {
                        epubIn = getAssets().open("fixture/kokka-to-kakumei-ch1.epub");
                    } catch (java.io.IOException e) {
                        // EPUB not present in this build
                        return;
                    }

                    // Read EPUB fully into memory
                    byte[] epubBytes;
                    try {
                        java.io.ByteArrayOutputStream baos = new java.io.ByteArrayOutputStream();
                        byte[] buf = new byte[65536];
                        int r;
                        while ((r = epubIn.read(buf)) > 0) {
                            baos.write(buf, 0, r);
                        }
                        epubBytes = baos.toByteArray();
                    } catch (java.io.IOException e) {
                        return;
                    } finally {
                        try { epubIn.close(); } catch (java.io.IOException ignored) {}
                    }

                    // Write the marker BEFORE creating anything
                    // (at-most-once): if the process dies after this
                    // point, the next launch will not create a second
                    // notebook. If the marker cannot be written, do
                    // not create the notebook at all.
                    java.io.FileOutputStream markerFos = null;
                    try {
                        markerFos = new java.io.FileOutputStream(marker);
                        markerFos.write(new byte[0]);
                    } catch (java.io.IOException e) {
                        return;
                    } finally {
                        if (markerFos != null) try { markerFos.close(); } catch (java.io.IOException ignored) {}
                    }

                    // Create notebook and add file source via Bridge
                    long nbId = Bridge.createNotebook("国家と革命（テスト）", System.currentTimeMillis() / 1000);
                    if (nbId > 0) {
                        Bridge.addFileSource(nbId, "国家と革命 第1章", "epub", epubBytes);
                    }
                } catch (Throwable t) {
                    // never crash
                }
            }
        }, new Runnable() {
            @Override
            public void run() {
                // no UI callback needed
            }
        });
    }
}
