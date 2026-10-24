%global debug_package %{nil}

Name:           smpl-calendar
Version:        %{calendar_version}
Release:        1%{?dist}
Summary:        Standalone calendar, clocks, weather and event reminders
License:        MIT AND OFL-1.1
URL:            https://github.com/smpl-os/smpl-apps
Source0:        smpl-calendar.tar.gz
ExclusiveArch:  x86_64

Requires:       libnotify
Requires:       systemd
Requires:       python3-dbus
Requires:       fontconfig%{?_isa}
Requires:       freetype%{?_isa}
Requires:       libxkbcommon%{?_isa}
Requires:       libxkbcommon-x11%{?_isa}
Requires:       libwayland-client%{?_isa}
Requires:       libwayland-cursor%{?_isa}
Requires:       libwayland-egl%{?_isa}
Requires:       libX11%{?_isa}
Requires:       libXcursor%{?_isa}
Requires:       libXi%{?_isa}
Requires:       libXrandr%{?_isa}
Requires:       libglvnd-egl%{?_isa}
Requires:       libglvnd-glx%{?_isa}

%description
The smplOS calendar as a standalone KDE-compatible application for Nobara 43.
Includes local SQLite events, recurring reminders, clocks and optional weather.
Neither smplOS nor Hyprland is required. Reminders start automatically at
desktop login and when the calendar is opened from the application menu.

%prep
%setup -q -n smpl-calendar

%build
# Binaries are built separately on Fedora 43, not copied from the Arch build.

%install
install -Dpm0755 smpl-calendar %{buildroot}%{_bindir}/smpl-calendar
install -Dpm0755 smpl-calendar-alertd %{buildroot}%{_bindir}/smpl-calendar-alertd
install -Dpm0644 smpl-calendar.desktop %{buildroot}%{_datadir}/applications/smpl-calendar.desktop
install -Dpm0644 smpl-calendar-reminders.desktop %{buildroot}%{_sysconfdir}/xdg/autostart/smpl-calendar-reminders.desktop
install -Dpm0644 smpl-calendar-alertd.service %{buildroot}%{_userunitdir}/smpl-calendar-alertd.service
install -Dpm0755 setup-kde-shortcut.py %{buildroot}%{_bindir}/smpl-calendar-kde-shortcut

%post
%systemd_user_post smpl-calendar-alertd.service

%preun
%systemd_user_preun smpl-calendar-alertd.service

%postun
%systemd_user_postun_with_restart smpl-calendar-alertd.service

%files
%license DSEG-LICENSE.txt
%{_bindir}/smpl-calendar
%{_bindir}/smpl-calendar-alertd
%{_datadir}/applications/smpl-calendar.desktop
%{_sysconfdir}/xdg/autostart/smpl-calendar-reminders.desktop
%{_userunitdir}/smpl-calendar-alertd.service
%{_bindir}/smpl-calendar-kde-shortcut
