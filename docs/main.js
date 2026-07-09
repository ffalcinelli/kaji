/* ===== NAV: scroll class + hamburger ===== */
const nav = document.getElementById('nav');
const hamburger = document.getElementById('nav-hamburger');
const navLinks = document.getElementById('nav-links');

window.addEventListener('scroll', () => {
  nav.classList.toggle('scrolled', window.scrollY > 20);
}, { passive: true });

hamburger.addEventListener('click', () => {
  const open = navLinks.classList.toggle('open');
  hamburger.setAttribute('aria-expanded', open);
});

// Close mobile menu when a link is clicked
navLinks.querySelectorAll('a').forEach(a => {
  a.addEventListener('click', () => {
    navLinks.classList.remove('open');
    hamburger.setAttribute('aria-expanded', 'false');
  });
});

/* ===== INSTALL TABS ===== */
const tabs = document.querySelectorAll('.tab-btn');
const panels = document.querySelectorAll('.tab-panel');

tabs.forEach(tab => {
  tab.addEventListener('click', () => {
    const target = tab.getAttribute('aria-controls');

    tabs.forEach(t => {
      t.classList.remove('tab-btn--active');
      t.setAttribute('aria-selected', 'false');
    });
    panels.forEach(p => p.classList.add('tab-panel--hidden'));

    tab.classList.add('tab-btn--active');
    tab.setAttribute('aria-selected', 'true');
    document.getElementById(target).classList.remove('tab-panel--hidden');
  });
});

/* ===== COPY BUTTONS ===== */
document.querySelectorAll('.copy-btn').forEach(btn => {
  btn.addEventListener('click', async () => {
    const code = btn.getAttribute('data-code');
    if (!code) return;

    try {
      await navigator.clipboard.writeText(code.replace(/&#10;/g, '\n'));
      const orig = btn.textContent;
      btn.textContent = '✓ Copied!';
      btn.classList.add('copied');
      setTimeout(() => {
        btn.textContent = orig;
        btn.classList.remove('copied');
      }, 2000);
    } catch {
      // Clipboard API not available – silently fail
    }
  });
});

/* ===== SCROLL-TRIGGERED FADE-IN ===== */
const observer = new IntersectionObserver(
  entries => {
    entries.forEach(entry => {
      if (entry.isIntersecting) {
        entry.target.classList.add('visible');
        observer.unobserve(entry.target);
      }
    });
  },
  { threshold: 0.1, rootMargin: '0px 0px -40px 0px' }
);

// Apply fade-in to key elements after DOM load
const ANIMATE_SELECTORS = [
  '.feature-card',
  '.command-card',
  '.resource-chip',
  '.secret-option',
  '.workflow-step',
  '.stat-item',
];

document.querySelectorAll(ANIMATE_SELECTORS.join(',')).forEach((el, i) => {
  el.classList.add('fade-in');
  el.style.transitionDelay = `${(i % 6) * 60}ms`;
  observer.observe(el);
});

/* ===== SMOOTH ANCHOR SCROLLING (accounts for fixed nav) ===== */
document.querySelectorAll('a[href^="#"]').forEach(anchor => {
  anchor.addEventListener('click', e => {
    const id = anchor.getAttribute('href').slice(1);
    const target = document.getElementById(id);
    if (!target) return;
    e.preventDefault();
    const top = target.getBoundingClientRect().top + window.scrollY - 80;
    window.scrollTo({ top, behavior: 'smooth' });
  });
});

/* ===== RESOURCE CHIPS: stagger entrance ===== */
document.querySelectorAll('.resource-chip').forEach((chip, i) => {
  chip.style.animationDelay = `${i * 40}ms`;
});

/* ===== TERMINAL TYPING ANIMATION ===== */
// Animate the cursor blinking in the terminal (already handled via CSS)
// Add a subtle entrance stagger for terminal lines
const termLines = document.querySelectorAll('.terminal-body code > span');
termLines.forEach((span, i) => {
  span.style.opacity = '0';
  span.style.animation = `none`;
  setTimeout(() => {
    span.style.transition = `opacity 0.3s ease`;
    span.style.opacity = '1';
  }, 800 + i * 80);
});
