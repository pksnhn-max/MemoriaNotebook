package jp.memorianotebook;

import android.app.Activity;
import android.content.res.Configuration;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Bundle;
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
        if (dark) {
            ground = Color.rgb(0x13, 0x13, 0x12);
            surface = Color.rgb(0x1E, 0x1E, 0x1C);
            ink = Color.rgb(0xF2, 0xF2, 0xED);
            sub = Color.rgb(0x9A, 0x9A, 0x94);
            hairline = Color.rgb(0x2E, 0x2E, 0x2B);
        } else {
            ground = Color.rgb(0xF7, 0xF7, 0xF2);
            surface = Color.rgb(0xFF, 0xFF, 0xFE);
            ink = Color.rgb(0x1A, 0x1A, 0x18);
            sub = Color.rgb(0x6E, 0x6E, 0x68);
            hairline = Color.rgb(0xE2, 0xE2, 0xDC);
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

        extractRuriAssets();
        try {
            String ortPath = getApplicationInfo().nativeLibraryDir
                    + "/libonnxruntime.so";
            if (!Bridge.init(getFilesDir().getAbsolutePath(), ortPath)) {
                screen.addView(text("核の初期化に失敗しました。", 15, ink, false, false));
                return;
            }
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

    private void clearScreen() {
        screen.removeAllViews();
        bottomBar.removeAllViews();
        scroll.scrollTo(0, 0);
    }

    /** A pill-shaped button in the design-2 tone (no default grey slab). */
    private Button pillBtn(String label) {
        Button b = new Button(this);
        b.setText(label);
        b.setTextColor(ink);
        GradientDrawable g = new GradientDrawable();
        g.setColor(hairline);
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

        screen.addView(text("生成の座", 13, sub, false, true), matchWrap(16));
        LinearLayout seat = column();
        seat.setBackground(cardBg());
        seat.setPadding(sdp(14), sdp(12), sdp(14), sdp(12));
        seat.addView(text("● ローカル（既定）", 15, ink, false, true));
        seat.addView(text("端末内のモデルで生成します。",
                12, sub, false, false));
        seat.addView(text("○ API（任意）", 15, ink, false, false), matchWrap(10));
        seat.addView(text("自分のエンドポイントとキーを差す席です。接続は次の更新で入ります。",
                12, sub, false, false));
        screen.addView(seat, matchWrap(8));

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
                            toast(raw[0].substring(4).trim());
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
                                toast(raw[0].substring(4).trim());
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
        TextView m = text(meta, 12, sub, false, false);
        LinearLayout.LayoutParams mp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        mp.topMargin = dp(18);
        m.setLayoutParams(mp);
        card.addView(m);
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
                    i == currentTab ? ink : sub, false, i == currentTab);
            tv.setGravity(Gravity.CENTER);
            if (i == currentTab) {
                GradientDrawable pill = new GradientDrawable();
                pill.setColor(hairline);
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
                TextView bubble = text(bodyText, 15, ink, false, false);
                GradientDrawable bg = new GradientDrawable();
                bg.setColor(hairline);
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
                // Citation markers [#id] render as small [id] chips.
                // The vessel has no Markdown renderer: the model's
                // emphasis markers would show literally, so they are
                // stripped here (display-only; the stored text and the
                // citation parsing upstream stay untouched).
                String shown = bodyText.replaceAll("\\[#(\\d+)\\]", "[$1]")
                        .replace("**", "")
                        .replace("__", "")
                        .replaceAll("(?m)^#{1,6} +", "");
                TextView a = text(shown, 15, ink, false, false);
                thread.addView(a, matchWrap(12));
                if (!cites.isEmpty()) {
                    TextView c = text("出典 " + cites.split(",").length + " 件",
                            12, sub, false, false);
                    thread.addView(c, matchWrap(4));
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
                        if (result[0] != null && result[0].startsWith("ERR:")) {
                            toast(result[0].substring(4).trim());
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
        Button addBtn = pillBtn("テキスト資料を追加");
        addBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                String t = srcTitle.getText().toString().trim();
                String b = srcBody.getText().toString();
                if (b.trim().isEmpty()) return;
                if (t.isEmpty()) t = "テキスト資料";
                Bridge.addTextSource(currentNb, t, b);
                showNotebook(currentNb, currentTitle, 0);
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
                            results.addView(text(raw[0].substring(4).trim(),
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
        card.addView(text("音声概要", 13, sub, false, false));
        TextView big = text("ソースを、二人の対話で聴く", 18, ink, true, true);
        LinearLayout.LayoutParams bp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        bp.topMargin = dp(6);
        big.setLayoutParams(bp);
        card.addView(big);
        TextView desc = text("選択中のソースから、対話形式の音声を生成します。"
                + "台本は API のモデルが書きます（未設定のときは設定へ案内します）。"
                + "生成機能は準備中です。", 13, sub, false, false);
        LinearLayout.LayoutParams dsp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        dsp.topMargin = dp(6);
        desc.setLayoutParams(dsp);
        card.addView(desc);
        Button gen = pillBtn("生成する");
        gen.setEnabled(false);
        card.addView(gen, matchWrap(10));
        body.addView(card, matchWrap(10));

        TextView done = text("生成済み", 14, ink, false, true);
        body.addView(done, matchWrap(16));
        TextView none = text("まだ音声概要はありません。", 13, sub, false, false);
        body.addView(none, matchWrap(6));
    }
}
