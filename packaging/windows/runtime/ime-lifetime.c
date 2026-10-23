// SPDX-License-Identifier: MIT
// Standalone Windows GTK regression. Only messages to its own HWND are sent.
#include <gtk/gtk.h>
#include <gdk/win32/gdkwin32.h>
#include <pango/pangocairo.h>
#include <windows.h>
#include <imm.h>
#include <string.h>

GType gtk_im_context_ime_get_type (void);

static void
finalized (gpointer data, GObject *object)
{
  *(gboolean *) data = TRUE;
}

static void
release_on_preedit (GtkIMContext *context, gpointer data)
{
  GtkIMContext **owner = data;
  g_clear_object (owner);
}

int
main (int argc, char **argv)
{
  g_assert_cmpint (argc, ==, 2);
  SetErrorMode (SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
  const char *mode = argv[1];
  g_setenv ("GTK_A11Y", "none", TRUE);
  g_setenv ("DBUS_SESSION_BUS_ADDRESS", "disabled:", TRUE);
  if (g_str_equal (mode, "fontconfig"))
    {
      PangoFontMap *map = pango_cairo_font_map_new_for_font_type (CAIRO_FONT_TYPE_FT);
      g_assert_nonnull (map);
      pango_cairo_font_map_set_default (PANGO_CAIRO_FONT_MAP (map));
      g_object_unref (map);
    }
  gtk_init ();
  wchar_t module[MAX_PATH];
  g_assert_cmpuint (GetModuleFileNameW (GetModuleHandleW (L"libgtk-4-1.dll"), module, MAX_PATH), >, 0);
  char *module_utf8 = g_utf16_to_utf8 ((gunichar2 *) module, -1, NULL, NULL, NULL);
  g_print ("Loaded GTK %u.%u.%u: %s\n", gtk_get_major_version (), gtk_get_minor_version (), gtk_get_micro_version (), module_utf8);
  g_free (module_utf8);
  GtkWidget *window = gtk_window_new ();
  GtkWidget *client = gtk_label_new ("Vietnamese input lifecycle");
  gtk_window_set_child (GTK_WINDOW (window), client);
  gtk_window_present (GTK_WINDOW (window));
  while (g_main_context_iteration (NULL, FALSE));
  GdkSurface *surface = gtk_native_get_surface (GTK_NATIVE (window));
  g_assert_nonnull (surface);
  HWND hwnd = gdk_win32_surface_get_handle (surface);
  HIMC input = ImmCreateContext ();
  g_assert_nonnull (input);
  HIMC previous = ImmAssociateContext (hwnd, input);
  GtkIMContext *context = g_object_new (gtk_im_context_ime_get_type (), NULL);
  gboolean destroyed = FALSE;
  g_object_weak_ref (G_OBJECT (context), finalized, &destroyed);
  gtk_im_context_set_client_widget (context, client);
  gtk_im_context_focus_in (context);

  if (g_str_equal (mode, "reentrant"))
    {
      g_signal_connect (context, "preedit-start", G_CALLBACK (release_on_preedit), &context);
      SendMessageW (hwnd, WM_IME_STARTCOMPOSITION, 0, 0);
      g_assert_null (context);
    }
  else if (g_str_equal (mode, "fontconfig") || g_str_equal (mode, "native-font"))
    {
      SendMessageW (hwnd, WM_IME_NOTIFY, IMN_SETOPENSTATUS, 0);
      gtk_im_context_focus_out (context);
      g_clear_object (&context);
    }
  else
    {
      if (g_str_equal (mode, "repeat-focus"))
        {
          for (int i = 0; i < 20; i++)
            gtk_im_context_focus_in (context);
          gtk_im_context_focus_out (context);
        }
      else if (g_str_equal (mode, "detach"))
        gtk_im_context_set_client_widget (context, NULL);
      else if (g_str_equal (mode, "unrealize"))
        {
          gtk_widget_set_visible (window, FALSE);
          gtk_widget_unrealize (window);
          gtk_im_context_focus_out (context);
        }
      else
        g_assert_cmpstr (mode, ==, "dispose");
      g_clear_object (&context);
    }
  g_assert_true (destroyed);
  // A live allocation in the same size class makes stale filter data observable.
  GTypeQuery query;
  g_type_query (gtk_im_context_ime_get_type (), &query);
  gpointer poison[128];
  for (int i = 0; i < 128; i++)
    poison[i] = g_malloc0 (query.instance_size);
  if (IsWindow (hwnd))
    {
      SendMessageW (hwnd, WM_NULL, 0, 0);
      ImmAssociateContext (hwnd, previous);
    }
  while (g_main_context_iteration (NULL, FALSE));
  gtk_window_destroy (GTK_WINDOW (window));
  ImmDestroyContext (input);
  for (int i = 0; i < 128; i++)
    g_free (poison[i]);
  g_print ("IME lifecycle passed: %s\n", mode);
  return 0;
}
