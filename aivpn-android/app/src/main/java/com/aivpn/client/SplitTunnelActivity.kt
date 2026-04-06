package com.aivpn.client

import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.widget.CheckBox
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.aivpn.client.databinding.ActivitySplitTunnelBinding
import com.google.android.material.tabs.TabLayout

class SplitTunnelActivity : AppCompatActivity() {

    private lateinit var binding: ActivitySplitTunnelBinding

    // Apps
    private lateinit var appAdapter: AppListAdapter
    private var allApps = listOf<AppEntry>()
    private var filteredApps = listOf<AppEntry>()
    private val excludedPackages = mutableSetOf<String>()
    private var hideSystemApps = true
    private var searchQuery = ""

    // Domains
    private lateinit var domainAdapter: DomainListAdapter
    private val excludedDomains = mutableListOf<String>()

    data class AppEntry(
        val name: String,
        val packageName: String,
        val isSystem: Boolean,
        val icon: android.graphics.drawable.Drawable
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivitySplitTunnelBinding.inflate(layoutInflater)
        setContentView(binding.root)

        excludedPackages.addAll(SecureStorage.loadExcludedApps(this))
        excludedDomains.addAll(SecureStorage.loadExcludedDomains(this))

        setupTabs()
        setupAppsPage()
        setupSitesPage()

        binding.btnBack.setOnClickListener { finish() }

        updateCounter()
        loadApps()
    }

    // ──────────── Tabs ────────────

    private fun setupTabs() {
        val tabApps = binding.tabLayout.newTab().setText(getString(R.string.split_tunnel_tab_apps))
        val tabSites = binding.tabLayout.newTab().setText(getString(R.string.split_tunnel_tab_sites))
        binding.tabLayout.addTab(tabApps)
        binding.tabLayout.addTab(tabSites)

        updateTabBadges()

        binding.tabLayout.addOnTabSelectedListener(object : TabLayout.OnTabSelectedListener {
            override fun onTabSelected(tab: TabLayout.Tab) {
                when (tab.position) {
                    0 -> {
                        binding.pageApps.visibility = View.VISIBLE
                        binding.pageSites.visibility = View.GONE
                    }
                    1 -> {
                        binding.pageApps.visibility = View.GONE
                        binding.pageSites.visibility = View.VISIBLE
                    }
                }
            }
            override fun onTabUnselected(tab: TabLayout.Tab) {}
            override fun onTabReselected(tab: TabLayout.Tab) {}
        })
    }

    private fun updateTabBadges() {
        val appCount = excludedPackages.size
        val domainCount = excludedDomains.size
        binding.tabLayout.getTabAt(0)?.text = if (appCount > 0)
            "${getString(R.string.split_tunnel_tab_apps)} ($appCount)"
        else
            getString(R.string.split_tunnel_tab_apps)
        binding.tabLayout.getTabAt(1)?.text = if (domainCount > 0)
            "${getString(R.string.split_tunnel_tab_sites)} ($domainCount)"
        else
            getString(R.string.split_tunnel_tab_sites)
    }

    // ──────────── Apps page ────────────

    private fun setupAppsPage() {
        appAdapter = AppListAdapter()
        binding.recyclerApps.layoutManager = LinearLayoutManager(this)
        binding.recyclerApps.adapter = appAdapter

        // Search
        binding.editSearch.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
            override fun afterTextChanged(s: Editable?) {
                searchQuery = s?.toString()?.trim()?.lowercase() ?: ""
                applyFilter()
            }
        })

        // Hide system apps checkbox
        binding.checkSystem.isChecked = hideSystemApps
        binding.checkSystem.setOnCheckedChangeListener { _, checked ->
            hideSystemApps = checked
            applyFilter()
        }

        // Select all checkbox
        binding.checkAll.setOnCheckedChangeListener(null)
        binding.checkAll.setOnClickListener {
            val shouldSelect = binding.checkAll.isChecked
            if (shouldSelect) {
                for (app in filteredApps) excludedPackages.add(app.packageName)
            } else {
                for (app in filteredApps) excludedPackages.remove(app.packageName)
            }
            saveApps()
            appAdapter.notifyDataSetChanged()
        }
    }

    private fun loadApps() {
        val pm = packageManager
        val ownPackage = packageName

        // MATCH_ALL was added in API 33 (Android 13)
        // For older versions, use 0 (default behavior)
        val flags = if (android.os.Build.VERSION.SDK_INT >= 33) {
            PackageManager.MATCH_ALL
        } else {
            0
        }

        allApps = pm.getInstalledApplications(flags)
            .filter { appInfo ->
                // Exclude own package
                appInfo.packageName != ownPackage
            }
            .map { appInfo ->
                AppEntry(
                    name = appInfo.loadLabel(pm).toString(),
                    packageName = appInfo.packageName,
                    isSystem = (appInfo.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                        && (appInfo.flags and ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) == 0,
                    icon = appInfo.loadIcon(pm)
                )
            }
            .distinctBy { it.packageName }
            .sortedWith(
                compareBy<AppEntry> { !excludedPackages.contains(it.packageName) }
                    .thenBy { it.name.lowercase() }
            )
        applyFilter()
    }

    private fun applyFilter() {
        filteredApps = allApps.filter { app ->
            val matchesSystem = !hideSystemApps || !app.isSystem
            val matchesSearch = searchQuery.isEmpty() ||
                app.name.lowercase().contains(searchQuery) ||
                app.packageName.lowercase().contains(searchQuery)
            matchesSystem && matchesSearch
        }
        appAdapter.notifyDataSetChanged()
        updateSelectAllCheckbox()
    }

    private fun updateSelectAllCheckbox() {
        if (filteredApps.isEmpty()) {
            binding.checkAll.isChecked = false
            return
        }
        val selectedCount = filteredApps.count { excludedPackages.contains(it.packageName) }
        binding.checkAll.setOnCheckedChangeListener(null)
        binding.checkAll.isChecked = selectedCount == filteredApps.size
        // Re-attach click listener (not checkedChange to avoid recursion)
        binding.checkAll.setOnClickListener {
            val shouldSelect = binding.checkAll.isChecked
            if (shouldSelect) {
                for (app in filteredApps) excludedPackages.add(app.packageName)
            } else {
                for (app in filteredApps) excludedPackages.remove(app.packageName)
            }
            saveApps()
            appAdapter.notifyDataSetChanged()
            updateSelectAllCheckbox()
        }
    }

    private fun saveApps() {
        SecureStorage.saveExcludedApps(this, excludedPackages)
        updateCounter()
        updateTabBadges()
    }

    // ──────────── Sites page ────────────

    private fun setupSitesPage() {
        domainAdapter = DomainListAdapter()
        binding.recyclerDomains.layoutManager = LinearLayoutManager(this)
        binding.recyclerDomains.adapter = domainAdapter

        binding.btnAddDomain.setOnClickListener { addDomain() }

        binding.editDomain.setOnEditorActionListener { _, actionId, _ ->
            if (actionId == EditorInfo.IME_ACTION_DONE) {
                addDomain()
                true
            } else false
        }
    }

    private fun addDomain() {
        val raw = binding.editDomain.text.toString().trim()
            .lowercase()
            .removePrefix("http://")
            .removePrefix("https://")
            .removeSuffix("/")
            .trim()

        if (raw.isEmpty()) return

        // Validate domain format
        val domainRegex = Regex("^([a-z0-9]([a-z0-9\\-]{0,61}[a-z0-9])?\\.)+[a-z]{2,}$")
        if (!domainRegex.matches(raw)) {
            Toast.makeText(this, getString(R.string.split_tunnel_domain_invalid), Toast.LENGTH_SHORT).show()
            return
        }

        if (excludedDomains.contains(raw)) {
            Toast.makeText(this, getString(R.string.split_tunnel_domain_exists), Toast.LENGTH_SHORT).show()
            return
        }

        excludedDomains.add(0, raw)
        saveDomains()
        domainAdapter.notifyDataSetChanged()
        binding.editDomain.text?.clear()
    }

    private fun saveDomains() {
        SecureStorage.saveExcludedDomains(this, excludedDomains)
        updateCounter()
        updateTabBadges()
    }

    // ──────────── Counter ────────────

    private fun updateCounter() {
        val total = excludedPackages.size + excludedDomains.size
        binding.textCounter.text = if (total > 0)
            getString(R.string.split_tunnel_bypass_count, total)
        else
            ""
    }

    // ──────────── App adapter ────────────

    inner class AppListAdapter : RecyclerView.Adapter<AppListAdapter.VH>() {

        inner class VH(parent: ViewGroup) : RecyclerView.ViewHolder(
            LayoutInflater.from(parent.context).inflate(R.layout.item_app, parent, false)
        ) {
            val icon: ImageView = itemView.findViewById(R.id.imgAppIcon)
            val name: TextView = itemView.findViewById(R.id.textAppName)
            val check: CheckBox = itemView.findViewById(R.id.checkExcluded)
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int) = VH(parent)
        override fun getItemCount() = filteredApps.size

        override fun onBindViewHolder(holder: VH, position: Int) {
            val app = filteredApps[position]
            holder.icon.setImageDrawable(app.icon)
            holder.name.text = app.name
            holder.check.setOnCheckedChangeListener(null)
            holder.check.isChecked = excludedPackages.contains(app.packageName)
            holder.check.setOnCheckedChangeListener { _, checked ->
                if (checked) excludedPackages.add(app.packageName)
                else excludedPackages.remove(app.packageName)
                saveApps()
                updateSelectAllCheckbox()
            }
            holder.itemView.setOnClickListener {
                holder.check.isChecked = !holder.check.isChecked
            }
        }
    }

    // ──────────── Domain adapter ────────────

    inner class DomainListAdapter : RecyclerView.Adapter<DomainListAdapter.VH>() {

        inner class VH(parent: ViewGroup) : RecyclerView.ViewHolder(
            LayoutInflater.from(parent.context).inflate(R.layout.item_domain, parent, false)
        ) {
            val domain: TextView = itemView.findViewById(R.id.textDomain)
            val delete: ImageButton = itemView.findViewById(R.id.btnDelete)
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int) = VH(parent)
        override fun getItemCount() = excludedDomains.size

        override fun onBindViewHolder(holder: VH, position: Int) {
            holder.domain.text = excludedDomains[position]
            holder.delete.setOnClickListener {
                val pos = holder.adapterPosition
                if (pos >= 0 && pos < excludedDomains.size) {
                    excludedDomains.removeAt(pos)
                    saveDomains()
                    notifyItemRemoved(pos)
                }
            }
        }
    }
}
