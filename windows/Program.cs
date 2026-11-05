// Where the app starts. Started by Windows for the Widgets board (the package's COM server), it's the widgets' provider
// and opens no window (Widgets.cs). Otherwise it's the window, one per person: a second start, as a widget's link
// opens it, hands its link to the first and ends.
using System;
using System.Linq;
using System.Threading;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Windows.ApplicationModel.Activation;

namespace One;

public static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        XamlCheckProcessRequirements();
        WinRT.ComWrappersSupport.InitializeComWrappers();
        if (args.Contains(Widgets.ServerArgument))
        {
            Widgets.Serve();
            return 0;
        }
        // Toasts' clicks and pushes, before asking how it was started; and started for one of those alone, it's done.
        Notes.Start();
        var activated = AppInstance.GetCurrent().GetActivatedEventArgs();
        if (Notes.Background(activated)) return 0;
        var main = AppInstance.FindOrRegisterForKey("main");
        if (!main.IsCurrent)
        {
            // The window comes to the front for the link this start was given, as this start would have.
            AllowSetForegroundWindow(main.ProcessId);
            // Handed over on another thread: this one's apartment would wait on itself.
            var handed = new Thread(() => main.RedirectActivationToAsync(activated).AsTask().Wait());
            handed.SetApartmentState(ApartmentState.MTA);
            handed.Start();
            handed.Join();
            return 0;
        }
        main.Activated += (_, e) => App.Opened(Link(e));
        App.Pending = Link(activated);
        Microsoft.UI.Xaml.Application.Start(p =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = new App();
        });
        return 0;
    }

    [System.Runtime.InteropServices.DllImport("Microsoft.ui.xaml.dll")] static extern void XamlCheckProcessRequirements();
    [System.Runtime.InteropServices.DllImport("user32.dll")] static extern bool AllowSetForegroundWindow(uint process);

    /// The link a start opens (`<cli>://open?…`, or a page at your One App's address), if it's one: Windows gives an
    /// address it opens here as a link, or as the start's argument; a toast, as its `open`.
    static Uri? Link(AppActivationArguments e) => e.Data switch
    {
        IProtocolActivatedEventArgs p when e.Kind == ExtendedActivationKind.Protocol => p.Uri,
        Microsoft.Windows.AppNotifications.AppNotificationActivatedEventArgs n when e.Kind == ExtendedActivationKind.AppNotification => Notes.Link(n),
        ILaunchActivatedEventArgs l when Uri.TryCreate(l.Arguments.Trim().Trim('"'), UriKind.Absolute, out var url) && url.Scheme == "https" => url,
        _ => null,
    };
}
