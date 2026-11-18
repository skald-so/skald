# Privacy policy

Last changed: 18 November 2026.

This policy covers the Skald cloud, `app.skald.so` and `api.skald.so`, and the website `skald.so` with its old address `docs.skald.so`. `dev.skald.so` is a test service under this policy too; its data may be deleted at any time. A self-hosted Skald server has its operator's policy, not this one.

## Who runs the cloud
The Skald maintainer. Write to [info@skald.so](mailto:info@skald.so).

## What the cloud keeps
- Message texts, files, task payloads and code are end-to-end encrypted: the cloud cannot read them.
- What the server sees, such as usernames, devices, spaces and IP addresses: [threat model, "The server sees"](threat-model.md#the-server-sees). What it does not see: ["The server does not see"](threat-model.md#the-server-does-not-see).

## Sign-in
### GitHub
- The cloud's GitHub OAuth App asks for no scope: it gets read-only access to public information only.
- GitHub sends your public profile.
- The cloud stores only your GitHub user ID and login, as your sign-in method. It drops GitHub's token once it has read the profile.

### Google
- The cloud asks Google for `openid` and `email` only.
- Google sends your account's ID, its email, whether Google verified the email, and its Workspace domain, if it has one.
- The cloud stores only the ID, as your sign-in method. It does not store the email, the ID token or Google's tokens.

### How the cloud uses it
- To sign you in.
- To link a sign-in method to your account.
- To make your first username: from your GitHub login, or from your email's part before `@`. The username names your personal space and is your handle in the spaces you create, which their members see. You can change it; a change does not rename your personal space or your handle in spaces that already exist.
- While sign-up is closed: to check your GitHub login, your verified Google email or your Google Workspace domain against the lists of who may sign up.

Your data is never sold, shared for ads or used to train models. The use of data from Google APIs follows the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including its Limited Use requirements.

## Cookies and browser storage
`app.skald.so` sets two cookies, only for sign-in:
- `skald_sign_in` ties a sign-in to the browser that started it. It goes when the sign-in completes, or after 10 minutes.
- `skald_session` keeps you signed in. It goes when you sign out, or after 30 days.

The website sets no cookies. It keeps your display and search settings in your browser's storage, which it sends nowhere.

There is no analytics, tracking or advertising on the website or in the app.

## Who else processes data
- GitHub hosts the website and runs GitHub sign-in: [GitHub General Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
- Google runs Google sign-in: [Google Privacy Policy](https://policies.google.com/privacy).
- A hosting provider runs the cloud's servers.
- A mail provider handles mail to `info@skald.so`.

## How long the cloud keeps data
- Your account and its username: until the account is deleted.
- A sign-in method: until you remove it or the account is deleted.
- A web session works for 30 days from its sign-in. Its record, with its browser, OS, start and last use, stays until the session is ended, its sign-in method is removed, the account signs in on the web after those 30 days, or the account is deleted.
- A device session: until it signs out, a web session of the account ends it, or the account is deleted.
- Encrypted messages, history and attachments: for the periods in [Cloud limits](dev/architecture.md#cloud-limits); attachments as long as their channel's messages ([Attachments](protocol/delivery.md#attachments)).
- Devices, spaces, rosters and the rest of what the server sees, beyond the above: until the account is deleted; a space's records until the space is deleted.
- Logs: the log store keeps about 14 days. Each service also keeps its own log, limited by size, not age, so a quiet service can keep older lines until they roll over. Logs hold no message contents; they can hold IP addresses and values from errors, such as a username.
- No access log of `app.skald.so` or `api.skald.so` is kept. `docs.skald.so`, which redirects to the website, logs each request: its IP address, user agent and path, in a log limited by size.
- Encrypted database backups: about 2 days.

## Deletion and other requests
To delete your account, or to ask what the cloud keeps about you, write to [info@skald.so](mailto:info@skald.so). It is done within 30 days.

## Changes
Each change updates the date at the top. Earlier versions: [the page's history on GitHub](https://github.com/skald-so/skald/commits/main/docs/privacy.md).
