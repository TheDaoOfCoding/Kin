using System;
using System.Drawing;
using System.Threading;
using System.Windows;
using System.Windows.Threading;
using Forms = System.Windows.Forms;

namespace Kin;

public partial class App : Application
{
    // Reverse-DNS app identifier; also used as the single-instance mutex name.
    private const string AppId = "app.kinrelay.desktop";

    private Mutex? _singleInstance;
    private Forms.NotifyIcon? _tray;
    private Forms.ContextMenuStrip? _menu;
    private Icon? _icon;
    private HelloWindow? _window;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        // A crash would otherwise leave a ghost icon in the tray until the mouse hovers over it.
        // Neither handler sets e.Handled or swallows anything: the crash still goes through.
        DispatcherUnhandledException += (_, _) => TryDisposeTray();
        AppDomain.CurrentDomain.UnhandledException += (_, _) => TryDisposeOnUiThread();

        _singleInstance = new Mutex(true, AppId, out bool isFirst);
        if (!isFirst)
        {
            Shutdown();
            return;
        }

        _menu = new Forms.ContextMenuStrip();
        _menu.Items.Add("Open", null, (_, _) => ShowWindow());
        _menu.Items.Add("Quit", null, (_, _) => Shutdown());

        // Load the size the tray actually draws, not the default 32x32 scaled down.
        using (var stream = GetResourceStream(new Uri("Assets/kin.ico", UriKind.Relative))!.Stream)
        {
            _icon = new Icon(stream, Forms.SystemInformation.SmallIconSize);
        }

        _tray = new Forms.NotifyIcon
        {
            Icon = _icon,
            Text = "Kin",
            ContextMenuStrip = _menu,
            Visible = true,
        };
        _tray.DoubleClick += (_, _) => ShowWindow();
    }

    private void ShowWindow()
    {
        // Created on open, destroyed on close: keeps idle memory low (see docs/windows.md).
        if (_window is null)
        {
            _window = new HelloWindow();
            _window.Closed += (_, _) => _window = null;
        }
        _window.Show();
        _window.Activate();
    }

    // The crash can come from any thread, but WinForms objects belong to the UI thread. Hand the
    // cleanup to it, and give up after a second if it is blocked or already gone.
    private void TryDisposeOnUiThread()
    {
        try
        {
            Dispatcher.Invoke(DisposeTray, DispatcherPriority.Send, CancellationToken.None, TimeSpan.FromSeconds(1));
        }
        catch
        {
            // Cleanup is best effort. Never let it hide the original crash.
        }
    }

    private void TryDisposeTray()
    {
        try
        {
            DisposeTray();
        }
        catch
        {
            // Same as above.
        }
    }

    private void DisposeTray()
    {
        // Hide first, otherwise Windows leaves a ghost icon until the mouse hovers over it.
        if (_tray is not null)
        {
            _tray.Visible = false;
            _tray.Dispose();
            _tray = null;
        }
        _menu?.Dispose();
        _menu = null;
        _icon?.Dispose();
        _icon = null;
    }

    protected override void OnExit(ExitEventArgs e)
    {
        DisposeTray();
        _singleInstance?.Dispose();
        base.OnExit(e);
    }
}
