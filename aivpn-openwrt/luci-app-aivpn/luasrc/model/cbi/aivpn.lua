local m, s, o

m = Map("aivpn", translate("AIVPN"),
    translate("Censorship-resistant VPN client with DPI evasion via traffic mimicry."))

s = m:section(TypedSection, "aivpn", translate("Connection"))
s.anonymous = true
s.addremove = false

o = s:option(Flag, "enabled", translate("Enabled"))
o.rmempty = false

o = s:option(Value, "connection_key", translate("Connection Key"),
    translate("Paste your aivpn:// key. Leave empty if using server+key below."))
o.password = true
o.rmempty = true

o = s:option(Value, "server", translate("Server Address"),
    translate("host:port — used only when connection_key is empty"))
o.rmempty = true
o.placeholder = "1.2.3.4:443"

o = s:option(Value, "server_key", translate("Server Public Key"),
    translate("Base64-encoded server public key"))
o.password = true
o.rmempty = true

return m
