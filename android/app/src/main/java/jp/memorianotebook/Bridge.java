package jp.memorianotebook;

/** JNI bridge to the Rust side (libnotebook_app.so). No logic here:
 *  every call is forwarded to notebook-core via notebook-app. */
public final class Bridge {
    static {
        System.loadLibrary("notebook_app");
    }

    private Bridge() {}

    public static native boolean init(String dataDir);
    public static native long createNotebook(String title);
    public static native long addTextSource(long notebookId, String title, String text);
    public static native String ask(long notebookId, String question);
}
