include $(TOPDIR)/rules.mk

PKG_NAME:=aivpn
PKG_VERSION:=0.8.0
PKG_RELEASE:=1

PKG_SOURCE_PROTO:=git
PKG_SOURCE_URL:=https://github.com/infosave2007/aivpn.git
PKG_SOURCE_VERSION:=master
PKG_MIRROR_HASH:=skip

include $(INCLUDE_DIR)/package.mk

define Package/aivpn
  SECTION:=net
  CATEGORY:=Network
  TITLE:=AIVPN censorship-resistant VPN client
  URL:=https://github.com/infosave2007/aivpn
  DEPENDS:=+kmod-tun
endef

define Package/aivpn/description
  Censorship-resistant VPN client with DPI evasion via traffic mimicry.
  Provides CLI client for OpenWrt routers.
endef

define Package/aivpn/conffiles
/etc/aivpn/client.json
endef

define Build/Compile
	cd $(PKG_BUILD_DIR) && \
	CARGO_HOME=$(STAGING_DIR)/cargo \
	cargo build --release --bin aivpn-client --target $(RUST_TARGET)
endef

define Package/aivpn/install
	$(INSTALL_DIR) $(1)/usr/bin
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/target/$(RUST_TARGET)/release/aivpn-client \
		$(1)/usr/bin/aivpn-client
	$(INSTALL_DIR) $(1)/etc/aivpn
	$(INSTALL_CONF) ./files/aivpn.config $(1)/etc/aivpn/client.json
	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/aivpn.init $(1)/etc/init.d/aivpn
	$(INSTALL_DIR) $(1)/etc/hotplug.d/iface
	$(INSTALL_DATA) ./files/aivpn.hotplug $(1)/etc/hotplug.d/iface/50-aivpn
endef

$(eval $(call BuildPackage,aivpn))
