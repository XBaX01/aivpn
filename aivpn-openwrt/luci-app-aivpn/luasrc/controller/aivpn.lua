module("luci.controller.aivpn", package.seeall)

function index()
    if not nixio.fs.access("/etc/config/aivpn") then return end

    local page = entry({"admin", "services", "aivpn"}, firstchild(), "AIVPN", 60)
    page.dependent = false

    entry({"admin", "services", "aivpn", "status"},
          template("aivpn/status"), "Status", 10).leaf = true

    entry({"admin", "services", "aivpn", "config"},
          cbi("aivpn"), "Configuration", 20).leaf = true

    entry({"admin", "services", "aivpn", "start"},
          call("action_start"), nil).leaf = true

    entry({"admin", "services", "aivpn", "stop"},
          call("action_stop"), nil).leaf = true
end

function action_start()
    luci.sys.call("/etc/init.d/aivpn start")
    luci.http.redirect(luci.dispatcher.build_url("admin/services/aivpn/status"))
end

function action_stop()
    luci.sys.call("/etc/init.d/aivpn stop")
    luci.http.redirect(luci.dispatcher.build_url("admin/services/aivpn/status"))
end
