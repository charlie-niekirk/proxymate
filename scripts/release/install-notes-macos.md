## Install on macOS

Proxymate isn't notarised by Apple yet, so macOS blocks it the first time you open it. You only have to allow it once; later versions install from inside the app (**Proxymate › Check for Updates…**).

1. Open `Proxymate_<version>_universal.dmg` and drag **Proxymate** to **Applications**.
2. Open Proxymate from Applications. macOS says it can't verify the app; click **Done** (or **OK**).
3. Open **System Settings › Privacy & Security**, scroll to the message about Proxymate, click **Open Anyway** and confirm with your password.

On macOS 13 and 14 you can instead Control-click the app in Applications, choose **Open**, then **Open** again.

Or, from Terminal:

```bash
xattr -dr com.apple.quarantine /Applications/Proxymate.app
```

The download is a universal app (Apple silicon and Intel), for macOS 13.3 or later.
