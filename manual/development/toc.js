// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded "><a href="index.html"><strong aria-hidden="true">1.</strong> Introduction</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="concept/resource.html"><strong aria-hidden="true">1.1.</strong> Resource</a></li></ol></li><li class="chapter-item expanded "><div><strong aria-hidden="true">2.</strong> Command Line</div></li><li><ol class="section"><li class="chapter-item expanded "><a href="cli/nixops4.html"><strong aria-hidden="true">2.1.</strong> nixops4</a></li><li class="chapter-item expanded "><a href="cli/nixops4-resource-runner.html"><strong aria-hidden="true">2.2.</strong> nixops4-resource-runner</a></li></ol></li><li class="chapter-item expanded "><a href="resource-provider/index.html"><strong aria-hidden="true">3.</strong> Resource Provider</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="resource-provider/interface.html"><strong aria-hidden="true">3.1.</strong> Resource Provider Interface</a></li><li class="chapter-item expanded "><a href="resource-provider/testing.html"><strong aria-hidden="true">3.2.</strong> Testing</a></li></ol></li><li class="chapter-item expanded "><a href="lib/index.html"><strong aria-hidden="true">4.</strong> Library</a></li><li class="chapter-item expanded "><a href="modules/index.html"><strong aria-hidden="true">5.</strong> Module Options</a></li><li class="chapter-item expanded "><div><strong aria-hidden="true">6.</strong> Protocols</div></li><li><ol class="section"><li class="chapter-item expanded "><a href="schema/resource-v0.html"><strong aria-hidden="true">6.1.</strong> Resource Protocol Schema</a></li></ol></li><li class="chapter-item expanded "><div><strong aria-hidden="true">7.</strong> Architecture</div></li><li><ol class="section"><li class="chapter-item expanded "><a href="architecture/nixops-components.html"><strong aria-hidden="true">7.1.</strong> Components</a></li></ol></li><li class="chapter-item expanded "><a href="index/index.html"><strong aria-hidden="true">8.</strong> Index</a></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString();
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
