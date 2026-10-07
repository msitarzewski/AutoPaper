using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;

namespace AutoPaper.Views;

/// <summary>A page that knows where keyboard focus belongs when it opens (its primary action, or the card the
/// person came back from).</summary>
internal interface IDefaultFocus
{
    /// <summary>The element to focus, or null for the page's first control.</summary>
    UIElement? DefaultFocusElement { get; }
}

/// <summary>A page with a level of its own that Back leaves first (Moods on a narrow window: a mood, then the list), so
/// the title bar's Back and Alt+Left go to the same place (WCAG 3.2.4; Fluent list/details: in stacked mode, Back goes
/// from the details to the list).</summary>
internal interface IInnerBack
{
    /// <summary>Back would stay in this page (it's showing its inner level).</summary>
    bool CanGoBackInside { get; }

    /// <summary>Raised when <see cref="CanGoBackInside"/> changes (the window enables or disables its Back).</summary>
    event EventHandler? CanGoBackInsideChanged;

    /// <summary>Goes back within the page; false when there's nowhere to go inside it.</summary>
    bool GoBackInside();
}

/// <summary>
/// Moves keyboard focus into a page that just opened, so it never falls back to the title bar (WCAG 2.4.3): the
/// page's own choice (<see cref="IDefaultFocus"/>), else the first control after a Settings pane's breadcrumb title
/// (the first setting, as Windows Settings does), else the page's first focusable element.
/// </summary>
internal static class PageFocus
{
    public static async Task FocusAsync(Page page, FocusState state)
    {
        if (!page.IsLoaded)
        {
            var loaded = new TaskCompletionSource();
            void OnLoaded(object sender, RoutedEventArgs args)
            {
                page.Loaded -= OnLoaded;
                loaded.TrySetResult();
            }
            page.Loaded += OnLoaded;
            await loaded.Task;
        }
        // After the layout pass that follows (a page that was collapsed, or whose controls were just shown).
        await NextIdleAsync(page.DispatcherQueue);
        foreach (var candidate in Candidates(page))
        {
            if ((await FocusManager.TryFocusAsync(candidate, state)).Succeeded)
            {
                return;
            }
        }
    }

    private static IEnumerable<DependencyObject> Candidates(Page page)
    {
        if (page is IDefaultFocus { DefaultFocusElement: { } chosen })
        {
            yield return chosen;
        }
        if (Find<BreadcrumbBar>(page) is { Parent: Panel panel } crumbs)
        {
            foreach (var sibling in panel.Children.SkipWhile(child => child != crumbs).Skip(1))
            {
                if (sibling.Visibility != Visibility.Visible || sibling is InfoBar)
                {
                    continue;
                }
                if (sibling is Control { IsTabStop: true, IsEnabled: true } control)
                {
                    yield return control;
                }
                else if (FocusManager.FindFirstFocusableElement(sibling) is { } inside)
                {
                    yield return inside;
                }
            }
        }
        if (FocusManager.FindFirstFocusableElement(page) is { } first)
        {
            yield return first;
        }
    }

    private static T? Find<T>(DependencyObject root) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T found)
            {
                return found;
            }
            if (Find<T>(child) is { } deeper)
            {
                return deeper;
            }
        }
        return null;
    }

    private static Task NextIdleAsync(DispatcherQueue queue)
    {
        var done = new TaskCompletionSource();
        if (!queue.TryEnqueue(DispatcherQueuePriority.Low, () => done.TrySetResult()))
        {
            done.TrySetResult();
        }
        return done.Task;
    }
}
