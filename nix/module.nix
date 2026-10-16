self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ryukaze;
  toml = pkgs.formats.toml { };
  configFile = toml.generate "ryukaze-config.toml" cfg.settings;

  # The package's policy lets everyone read and only root write; this one
  # opens the setters to the listed users.
  usersPolicy = pkgs.writeTextDir "share/dbus-1/system.d/io.github.kontonkara.ryukaze.users.conf" ''
    <!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
     "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
    <busconfig>
    ${lib.concatMapStrings (user: ''
      <policy user="${user}">
        <allow send_destination="io.github.kontonkara.ryukaze"/>
      </policy>
    '') cfg.users}
    </busconfig>
  '';
in
{
  options = {
    services = {
      ryukaze = {
        enable = lib.mkEnableOption "ryukazed, the MSI laptop EC daemon (needs the upstream msi-ec driver)";

        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          description = "The ryukaze package.";
        };

        users = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "alice" ];
          description = "Users allowed to change the EC over D-Bus. Everyone may read its status.";
        };

        tray = {
          enable = lib.mkEnableOption ''
            the ryukaze tray icon in graphical sessions. It starts with the
            session, hidden; a click or another start of ryukaze opens the
            window
          '';
        };

        settings = lib.mkOption {
          inherit (toml) type;
          default = { };
          example = {
            charge_limit = 80;
            profiles = {
              ac = {
                shift_mode = "turbo";
                fan_mode = "auto";
                super_battery = false;
              };
              battery = {
                shift_mode = "eco";
                super_battery = true;
              };
            };
          };
          description = ''
            ryukazed's /etc/ryukaze/config.toml; unset keys leave the EC alone.
            A profile is applied when the laptop goes on AC or on battery, and
            the charge limit at start. The EC keeps the charge limit across
            reboots, so unsetting it doesn't lift it: set 100.
          '';
        };
      };
    };
  };

  config = lib.mkIf cfg.enable {
    environment = {
      etc = {
        "ryukaze/config.toml".source = configFile;
      };

      # The window and its launcher entry.
      systemPackages = [ cfg.package ];
    };

    services = {
      dbus = {
        packages = [
          cfg.package
          usersPolicy
        ];
      };
    };

    systemd = {
      user = {
        services = {
          ryukaze = lib.mkIf cfg.tray.enable {
            description = "ryukaze tray icon";
            partOf = [ "graphical-session.target" ];
            after = [ "graphical-session.target" ];
            wantedBy = [ "graphical-session.target" ];

            # No wait for the panel: the icon registers whenever a
            # StatusNotifierWatcher shows up.
            serviceConfig = {
              ExecStart = "${cfg.package}/bin/ryukaze --hidden";
              Restart = "on-failure";
            };
          };
        };
      };

      services = {
        ryukazed = {
          description = "MSI laptop EC daemon";
          wantedBy = [ "multi-user.target" ];
          # A late msi-ec still gets its settings: the daemon reapplies them
          # when the driver binds.
          after = [ "systemd-modules-load.service" ];
          restartTriggers = [ configFile ];

          serviceConfig = {
            Type = "dbus";
            BusName = "io.github.kontonkara.ryukaze";
            ExecStart = "${cfg.package}/bin/ryukazed";
            Restart = "on-failure";

            # Writes go to /sys only; uevents come over netlink.
            ProtectSystem = "strict";
            ProtectHome = true;
            PrivateTmp = true;
            NoNewPrivileges = true;
            RestrictAddressFamilies = [
              "AF_UNIX"
              "AF_NETLINK"
            ];
            LockPersonality = true;
            MemoryDenyWriteExecute = true;
            SystemCallArchitectures = "native";
          };
        };
      };
    };
  };
}
