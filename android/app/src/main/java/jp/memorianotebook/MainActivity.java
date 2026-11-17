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

    // ---------- small helpers ----------

    private int dp(int v) {
        return (int) (v * getResources().getDisplayMetrics().density);
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
        return new SimpleDateFormat("M月d日", Locale.JAPAN)
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
        screen.setPadding(dp(16), dp(12), dp(16), dp(16));

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

        try {
            if (!Bridge.init(getFilesDir().getAbsolutePath())) {
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
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        out.putLong("nb", currentNb);
        out.putString("title", currentTitle);
        out.putInt("tab", currentTab);
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

        LinearLayout head = new LinearLayout(this);
        head.setOrientation(LinearLayout.HORIZONTAL);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = text("ノート", 26, ink, false, true);
        title.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        head.addView(title);
        head.addView(text("⌕", 22, ink, false, false));
        TextView dots = text("⋮", 22, ink, false, false);
        LinearLayout.LayoutParams dp2 = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT);
        dp2.leftMargin = dp(14);
        dots.setLayoutParams(dp2);
        head.addView(dots);
        screen.addView(head);

        TextView tabs = text("すべて    最近    ダウンロード済み", 14, sub, false, false);
        screen.addView(tabs, matchWrap(14));

        // New-notebook row (the mock's "＋ 新しいノートを作成" card):
        // one row — title field and a small create pill side by side.
        LinearLayout createCard = new LinearLayout(this);
        createCard.setOrientation(LinearLayout.HORIZONTAL);
        createCard.setGravity(Gravity.CENTER_VERTICAL);
        createCard.setBackground(cardBg());
        createCard.setPadding(dp(14), dp(6), dp(10), dp(6));
        final EditText newTitle = new EditText(this);
        newTitle.setHint("＋ 新しいノートを作成");
        newTitle.setTextColor(ink);
        newTitle.setHintTextColor(sub);
        newTitle.setBackgroundColor(Color.TRANSPARENT);
        newTitle.setLayoutParams(new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        createCard.addView(newTitle);
        Button createBtn = pillBtn("作成");
        createBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                String t = newTitle.getText().toString().trim();
                if (t.isEmpty()) t = "無題のノート";
                long id = Bridge.createNotebook(t, System.currentTimeMillis() / 1000L);
                if (id >= 0) showNotebook(id, t, 1);
                else showList();
            }
        });
        createCard.addView(createBtn);
        screen.addView(createCard, matchWrap(14));

        String rawList = Bridge.listNotebooks();
        if (rawList != null && rawList.startsWith("ERR:")) {
            screen.addView(text(rawList, 14, sub, false, false), matchWrap(14));
        }
        String[][] nbs = lines(rawList);
        if (nbs.length == 0) {
            TextView empty = text("まだノートがありません。上の欄から作ってください。",
                    14, sub, false, false);
            screen.addView(empty, matchWrap(18));
            return;
        }
        // Two-column card grid, built as rows of two.
        for (int i = 0; i < nbs.length; i += 2) {
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.HORIZONTAL);
            row.addView(notebookCard(nbs[i]));
            if (i + 1 < nbs.length) {
                row.addView(notebookCard(nbs[i + 1]));
            } else {
                View spacer = new View(this);
                spacer.setLayoutParams(new LinearLayout.LayoutParams(0,
                        LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
                row.addView(spacer);
            }
            screen.addView(row, matchWrap(12));
        }
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
        String meta = srcCount + " ソース"
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
        clearScreen();

        LinearLayout head = new LinearLayout(this);
        head.setOrientation(LinearLayout.HORIZONTAL);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView back = text("←", 20, ink, false, false);
        back.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showList();
            }
        });
        head.addView(back);
        TextView t = text(title, 17, ink, true, true);
        LinearLayout.LayoutParams tp = new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f);
        tp.leftMargin = dp(10);
        t.setLayoutParams(tp);
        head.addView(t);
        head.addView(text("⌕", 20, ink, false, false));
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
        card.setPadding(dp(14), dp(12), dp(14), dp(12));
        card.addView(text(currentTitle, 18, ink, true, true));
        TextView meta = text(srcCount + " ソース", 12, sub, false, false);
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
                String shown = bodyText.replaceAll("\\[#(\\d+)\\]", "[$1]");
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
                String q = input.getText().toString().trim();
                if (q.isEmpty()) return;
                Bridge.ask(currentNb, q);
                showNotebook(currentNb, currentTitle, 1);
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
