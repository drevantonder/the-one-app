// The app: one window. The install's accent is the controls' accent, in place of Windows' own.
using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Windows.UI;

namespace One;

public partial class App : Application
{
    public static MainWindow? Window { get; private set; }

    public App()
    {
        InitializeComponent();
        // What went wrong, if anything does, kept where the person (or whoever helps them) can send it.
        UnhandledException += (_, e) => Problem(e.Exception);
        AppDomain.CurrentDomain.UnhandledException += (_, e) => Problem(e.ExceptionObject as Exception);
        TaskScheduler.UnobservedTaskException += (_, e) => Problem(e.Exception);
        // Each app screen's WebView2 keeps its storage with this app's, not beside its program.
        Environment.SetEnvironmentVariable("WEBVIEW2_USER_DATA_FOLDER", System.IO.Path.Combine(Brand.Folder, "Screens"));
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            Accent(Look.Hex(Brand.Accent));
            // Pages and screens sit on the window's backdrop alike, as the Mac app's do on its window.
            Resources["NavigationViewContentBackground"] = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            Resources["NavigationViewContentGridBorderBrush"] = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            Window = new MainWindow();
            Window.Activate();
        }
        catch (Exception e)
        {
            Problem(e);
            throw;
        }
    }

    public static void Problem(Exception? e)
    {
        try
        {
            System.IO.Directory.CreateDirectory(Brand.Folder);
            System.IO.File.AppendAllText(System.IO.Path.Combine(Brand.Folder, "What went wrong.txt"), $"{DateTime.Now:u} {e}\n\n");
        }
        catch { }
    }

    /// The accent and its lighter and darker shades, as Windows makes its own.
    void Accent(Color c)
    {
        Color Mix(double t, byte to) => Color.FromArgb(255, (byte)(c.R + (to - c.R) * t), (byte)(c.G + (to - c.G) * t), (byte)(c.B + (to - c.B) * t));
        Resources["SystemAccentColor"] = c;
        for (var i = 1; i <= 3; i++)
        {
            Resources[$"SystemAccentColorLight{i}"] = Mix(0.18 * i, 255);
            Resources[$"SystemAccentColorDark{i}"] = Mix(0.18 * i, 0);
        }
    }
}
