// The main page: the web UI's theme with the fonts bundled from npm packages, the page's styles, and
// Copy, which puts its command on the clipboard and says so to a screen reader.
import '@fontsource-variable/inter';
import '@fontsource-variable/space-grotesk';
import '@fontsource-variable/jetbrains-mono';
import '../../web/src/theme/theme.css';
import './main.css';

const status = document.querySelector('.copied');
for (const button of document.querySelectorAll('.copy')) {
  const label = button.querySelector('.copy-label');
  let timer;
  button.addEventListener('click', async () => {
    const command = button.closest('.command').querySelector('code').textContent;
    let copied = true;
    try {
      await navigator.clipboard.writeText(command);
    } catch {
      copied = false;
    }
    status.textContent = copied ? `Copied: ${command}` : 'Copy failed: select the command and copy it.';
    label.textContent = copied ? 'Copied' : 'Copy';
    clearTimeout(timer);
    timer = setTimeout(() => {
      label.textContent = 'Copy';
      status.textContent = '';
    }, 2000);
  });
}
