// The app: one window, in the person's own accent colour, as Windows' apps are.
using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Windows.UI;

namespace One;

public partial class App : Application
{
    public static MainWindow? Window { get; private set; }
    /// The link this start opens, until the window opens it.
    public static Uri? Pending { get; set; }

    public App()
    {
        InitializeComponent();
        // What went wrong, if anything does, kept where the person (or whoever helps them) can send it.
        UnhandledException += (_, e) => Problem(e.Exception);
        AppDomain.CurrentDomain.UnhandledException += (_, e) => Problem(e.ExceptionObject as Exception);
        TaskScheduler.UnobservedTaskException += (_, e) => Problem(e.Exception);
        // Each app screen's WebView2 keeps its storage with this app's, not beside its program.
        Environment.SetEnvironmentVariable("WEBVIEW2_USER_DATA_FOLDER", System.IO.Path.Combine(WhiteLabel.Folder, "Screens"));
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            // Pages and screens sit on the window's backdrop alike, as the Mac app's do on its window.
            Resources["NavigationViewContentBackground"] = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            Resources["NavigationViewContentGridBorderBrush"] = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            Window = new MainWindow();
            Window.Activate();
            if (Pending is { } link) Window.Open(link);
            Pending = null;
        }
        catch (Exception e)
        {
            Problem(e);
            throw;
        }
    }

    /// A later start's link (a widget's), opened in this window, brought to the front.
    public static void Opened(Uri? link) => Window?.DispatcherQueue.TryEnqueue(() =>
    {
        Window.Activate();
        Window.Bring();
        if (link != null) Window.Open(link);
    });

    public static void Problem(Exception? e)
    {
        try
        {
            System.IO.Directory.CreateDirectory(WhiteLabel.Folder);
            System.IO.File.AppendAllText(System.IO.Path.Combine(WhiteLabel.Folder, "What went wrong.txt"), $"{DateTime.Now:u} {e}\n\n");
        }
        catch { }
    }
}
