// Lightweight, dependency-free i18n. Two locales (en, fr), flat key
// namespace, `{name}`-style interpolation. Adding a locale means adding a
// sibling object under `MESSAGES`; no build-time codegen.
//
// Convention: keys are `area.subarea.purpose`. Pluralization: pass a
// numeric `count` param and provide `key.one` / `key.other` entries —
// translate() picks the form via Intl.PluralRules and falls back to the
// bare key when no plural forms exist.

export const SUPPORTED_LANGUAGES = ['en', 'fr'];
export const DEFAULT_LANGUAGE = 'en';

export const LANGUAGE_LABELS = {
  en: 'English',
  fr: 'Français',
};

const MESSAGES = {
  en: {
    'app.loading': 'Loading…',

    'view.tasks': 'Tasks',
    'view.history': 'History',
    'view.statistics': 'Statistics',
    'view.settings': 'Settings',

    'sidebar.aria': 'Sidebar',
    'sidebar.search': 'Search',
    'sidebar.section.library': 'Library',
    'sidebar.section.application': 'Application',
    'sidebar.brand.version': 'Version {version}',

    'toolbar.toggle.show': 'Show sidebar',
    'toolbar.toggle.hide': 'Hide sidebar',

    'common.cancel': 'Cancel',
    'common.save': 'Save',
    'common.delete': 'Delete',
    'common.restore': 'Restore',
    'common.reveal': 'Reveal',
    'common.modify': 'Modify',
    'common.choose': 'Choose…',
    'common.clear': 'Clear',
    'common.open': 'Open',
    'common.ok': 'OK',
    'common.never': 'Never',
    'common.empty': 'Empty',
    'common.notset': 'Not set',
    'common.dash': '—',
    'common.backup': 'Backup',
    'common.task': 'Task',
    'common.unlimited': 'All',

    'home.new_task': 'New task',
    'task.last_run': 'Last run {time} · {schedule}',
    'task.schedule.manual': 'Manual',
    'task.schedule.hourly': 'Hourly',
    'task.schedule.daily': 'Daily',
    'task.schedule.weekly': 'Weekly',
    'task.schedule.monthly': 'Monthly',
    'task.action.backup': 'Back up',
    'task.action.stop': 'Stop',
    'task.action.modify': 'Modify',
    'task.action.delete': 'Delete',
    'task.aria.run': 'Run {name}',
    'task.aria.cancel': 'Cancel {name}',
    'task.aria.modify': 'Modify {name}',
    'task.aria.delete': 'Delete {name}',
    'task.aria.progress': 'Backup progress for {name}',
    'task.toast.added': 'Task added',
    'task.toast.updated': 'Task updated',
    'task.confirm.delete.title': 'Delete this task?',
    'task.confirm.delete.body': 'Existing backup folders will not be removed.',
    'task.confirm.backup.title': 'Back up “{name}”?',
    'task.confirm.backup.body': 'From: {source}\nTo: {destination}',
    'task.confirm.backup.action': 'Start Backup',

    'form.title.new': 'New Task',
    'form.title.edit': 'Modify Task',
    'form.label.name': 'Name',
    'form.label.source': 'Source',
    'form.label.destination': 'Destination',
    'form.label.destination_default': 'Destination (default available)',
    'form.label.schedule': 'Schedule',
    'form.placeholder.name': 'Documents — April',
    'form.placeholder.choose': 'Choose folder…',
    'form.hint.schedule': 'Automatic runs require driveby to be open',
    'form.action.add': 'Add Task',
    'form.action.save': 'Save',
    'form.error.name': 'Task name required',
    'form.error.source': 'Source folder required',
    'form.error.dest': 'Destination required',
    'form.dialog.select_source': 'Select source folder',
    'form.dialog.select_destination': 'Select destination',

    'backup.toast.complete': 'Backup complete',
    'backup.toast.cancelled': 'Backup cancelled',
    'backup.toast.failed': 'Backup failed: {error}',
    'backup.notification.title': 'driveby',
    'backup.notification.body': 'Backup of “{name}” complete',

    'restore.dialog.select': 'Select restore destination',
    'restore.dialog.title': 'Restore this backup?',
    'restore.dialog.body': 'Restoring the backup:\n{source}\n\nFiles will be written to:\n{destination}',
    'restore.dialog.action': 'Restore',
    'restore.toast.success.one': 'Restored 1 file',
    'restore.toast.success.other': 'Restored {n} files',
    'restore.toast.failed': 'Restore failed: {error}',
    'restore.toast.cancelled': 'Restore cancelled',
    'restore.busy': 'A restore is already running',
    'restore.progress.title': 'Restoring…',
    'restore.progress.starting': 'Preparing…',
    'restore.action.stop': 'Stop',

    'reveal.cannot_open': 'Cannot open: {error}',

    'history.title': 'History',
    'history.clear_all': 'Clear All',
    'history.search': 'Search…',
    'history.filter.aria': 'Filter status',
    'history.filter.all': 'All',
    'history.filter.success': 'Success',
    'history.filter.errors': 'Errors',
    'history.filter.cancelled': 'Cancelled',
    'history.col.date': 'Date',
    'history.col.task': 'Task',
    'history.col.status': 'Status',
    'history.col.size': 'Size',
    'history.col.files': 'Files',
    'history.col.duration': 'Duration',
    'history.col.actions': 'Actions',
    'history.status.success': 'Success',
    'history.status.cancelled': 'Cancelled',
    'history.status.error': 'Error',
    'history.unreadable.one': '1 source item could not be read — its copy was left untouched in the destination',
    'history.unreadable.other': '{n} source items could not be read — their copies were left untouched in the destination',
    'history.confirm.clear.title': 'Clear all history?',
    'history.confirm.clear.body': 'Entries will be removed. Existing backup folders are untouched.',
    'history.confirm.clear.action': 'Clear',

    'statistics.backed_up': 'Backed Up',
    'statistics.tasks': 'Tasks',
    'statistics.successful_runs': 'Successful Runs',
    'statistics.aria.day': 'Backups on {day}: {bytes}',
    'statistics.aria.bars': 'Successes vs errors per task',
    'chart.empty.backups': 'No backups yet',
    'chart.empty.tasks': 'No tasks',
    'chart.legend.success': 'Success',
    'chart.legend.error': 'Error',

    'settings.section.general': 'General',
    'settings.section.options': 'Backup Options',
    'settings.section.filtering': 'Filtering',
    'settings.section.appearance': 'Appearance',
    'settings.section.language': 'Language',
    'settings.section.background': 'Background',
    'settings.section.updates': 'Updates',
    'settings.section.diagnostics': 'Diagnostics',

    'settings.label.close_to_tray': 'Keep running when the window is closed',
    'settings.label.autostart': 'Start driveby at login',
    'settings.label.version': 'Version',
    'settings.label.check_updates_on_start': 'Check for updates at launch',
    'settings.tip.close_to_tray': "Keeps driveby running in the notification area when you close the window, so scheduled backups still fire.",
    'settings.tip.autostart': "Starts driveby in the background when you log in, so scheduled backups run without opening the app.",
    'settings.toast.autostart_failed': 'Could not change the startup setting',

    'updates.up_to_date': 'driveby is up to date',
    'updates.available': 'Version {version} is available',
    'updates.installing': 'Downloading the update — driveby will restart on its own',
    'updates.action.installing': 'Installing…',
    'updates.action.check': 'Check for updates',
    'updates.action.checking': 'Checking…',
    'updates.action.install': 'Install and restart',
    'updates.toast.available': 'Update available — see Settings',
    'updates.toast.failed': 'Update check failed: {error}',

    'settings.label.default_dest': 'Default destination',
    'settings.dialog.default_dest': 'Select default destination',
    'settings.label.confirm_backup': 'Confirm before each backup',
    'settings.label.notifications': 'System notifications',
    'settings.label.verify': 'Verify after copy',
    'settings.label.continue_on_error': 'Continue on error',
    'settings.label.preserve_mtime': 'Preserve file modification time',
    'settings.label.parallel_copies': 'Files copied at once',
    'settings.label.history_retention': 'History kept',
    'settings.label.exclude': 'Exclude patterns',
    'settings.label.appearance': 'Appearance',
    'settings.label.language': 'Language',
    'settings.label.logs': 'Application logs',

    'settings.tip.verify': "Reads every copied file back and compares it against a fingerprint taken while writing, to catch corruption.",
    'settings.tip.parallel_copies': "How many files are copied at once. 4 suits SSDs and network drives; use 1 for a spinning hard disk.",
    'settings.tip.history_retention': "How far back the history goes. Older runs are dropped; backup folders are never touched.",
    'settings.tip.continue_on_error': "Skips files that cannot be copied — locked, or not permitted — and carries on instead of stopping the whole job.",
    'settings.tip.preserve_mtime': "Keeps each file’s original modification date, which lets later runs skip unchanged files instantly.",
    'settings.tip.exclude': "One pattern per line, or comma-separated. * matches any characters, ** crosses folders, ? a single one, and a leading ! puts something back in.",

    'settings.placeholder.exclude': '*.tmp\nnode_modules\n.DS_Store\n!important.tmp',

    'settings.retention.1d': '1 day',
    'settings.retention.1w': '1 week',
    'settings.retention.1m': '1 month',
    'settings.retention.1y': '1 year',

    'settings.theme.light': 'Light',
    'settings.theme.dark': 'Dark',
    'settings.theme.system': 'System',

    'settings.toast.cannot_open_logs': 'Cannot open logs: {error}',
  },

  fr: {
    'app.loading': 'Chargement…',

    'view.tasks': 'Tâches',
    'view.history': 'Historique',
    'view.statistics': 'Statistiques',
    'view.settings': 'Paramètres',

    'sidebar.aria': 'Barre latérale',
    'sidebar.search': 'Rechercher',
    'sidebar.section.library': 'Bibliothèque',
    'sidebar.section.application': 'Application',
    'sidebar.brand.version': 'Version {version}',

    'toolbar.toggle.show': 'Afficher la barre latérale',
    'toolbar.toggle.hide': 'Masquer la barre latérale',

    'common.cancel': 'Annuler',
    'common.save': 'Enregistrer',
    'common.delete': 'Supprimer',
    'common.restore': 'Restaurer',
    'common.reveal': 'Afficher',
    'common.modify': 'Modifier',
    'common.choose': 'Choisir…',
    'common.clear': 'Effacer',
    'common.open': 'Ouvrir',
    'common.ok': 'OK',
    'common.never': 'Jamais',
    'common.empty': 'Vide',
    'common.notset': 'Non défini',
    'common.dash': '—',
    'common.backup': 'Sauvegarde',
    'common.task': 'Tâche',
    'common.unlimited': 'Tout',

    'home.new_task': 'Nouvelle tâche',
    'task.last_run': 'Dernière exécution {time} · {schedule}',
    'task.schedule.manual': 'Manuel',
    'task.schedule.hourly': 'Toutes les heures',
    'task.schedule.daily': 'Quotidien',
    'task.schedule.weekly': 'Hebdomadaire',
    'task.schedule.monthly': 'Mensuel',
    'task.action.backup': 'Sauvegarder',
    'task.action.stop': 'Arrêter',
    'task.action.modify': 'Modifier',
    'task.action.delete': 'Supprimer',
    'task.aria.run': 'Lancer {name}',
    'task.aria.cancel': 'Annuler {name}',
    'task.aria.modify': 'Modifier {name}',
    'task.aria.delete': 'Supprimer {name}',
    'task.aria.progress': 'Progression de la sauvegarde de {name}',
    'task.toast.added': 'Tâche ajoutée',
    'task.toast.updated': 'Tâche mise à jour',
    'task.confirm.delete.title': 'Supprimer cette tâche ?',
    'task.confirm.delete.body': 'Les dossiers de sauvegarde existants ne seront pas supprimés.',
    'task.confirm.backup.title': 'Sauvegarder « {name} » ?',
    'task.confirm.backup.body': 'Depuis : {source}\nVers : {destination}',
    'task.confirm.backup.action': 'Lancer la sauvegarde',

    'form.title.new': 'Nouvelle tâche',
    'form.title.edit': 'Modifier la tâche',
    'form.label.name': 'Nom',
    'form.label.source': 'Source',
    'form.label.destination': 'Destination',
    'form.label.destination_default': 'Destination (par défaut disponible)',
    'form.label.schedule': 'Planification',
    'form.placeholder.name': 'Documents — Avril',
    'form.placeholder.choose': 'Choisir un dossier…',
    'form.hint.schedule': "Les exécutions automatiques nécessitent que driveby soit ouvert",
    'form.action.add': 'Ajouter',
    'form.action.save': 'Enregistrer',
    'form.error.name': 'Le nom de la tâche est requis',
    'form.error.source': 'Le dossier source est requis',
    'form.error.dest': 'La destination est requise',
    'form.dialog.select_source': 'Sélectionner le dossier source',
    'form.dialog.select_destination': 'Sélectionner la destination',

    'backup.toast.complete': 'Sauvegarde terminée',
    'backup.toast.cancelled': 'Sauvegarde annulée',
    'backup.toast.failed': 'Échec de la sauvegarde : {error}',
    'backup.notification.title': 'driveby',
    'backup.notification.body': 'Sauvegarde de « {name} » terminée',

    'restore.dialog.select': 'Sélectionner la destination de restauration',
    'restore.dialog.title': 'Restaurer cette sauvegarde ?',
    'restore.dialog.body': 'Restauration de la sauvegarde :\n{source}\n\nLes fichiers seront écrits dans :\n{destination}',
    'restore.dialog.action': 'Restaurer',
    'restore.toast.success.one': '1 fichier restauré',
    'restore.toast.success.other': '{n} fichiers restaurés',
    'restore.toast.failed': 'Échec de la restauration : {error}',
    'restore.toast.cancelled': 'Restauration annulée',
    'restore.busy': 'Une restauration est déjà en cours',
    'restore.progress.title': 'Restauration…',
    'restore.progress.starting': 'Préparation…',
    'restore.action.stop': 'Arrêter',

    'reveal.cannot_open': 'Impossible d’ouvrir : {error}',

    'history.title': 'Historique',
    'history.clear_all': 'Tout effacer',
    'history.search': 'Rechercher…',
    'history.filter.aria': 'Filtrer par statut',
    'history.filter.all': 'Tous',
    'history.filter.success': 'Succès',
    'history.filter.errors': 'Erreurs',
    'history.filter.cancelled': 'Annulés',
    'history.col.date': 'Date',
    'history.col.task': 'Tâche',
    'history.col.status': 'Statut',
    'history.col.size': 'Taille',
    'history.col.files': 'Fichiers',
    'history.col.duration': 'Durée',
    'history.col.actions': 'Actions',
    'history.status.success': 'Succès',
    'history.status.cancelled': 'Annulé',
    'history.status.error': 'Erreur',
    'history.unreadable.one': "1 élément source illisible — sa copie a été laissée intacte dans la destination",
    'history.unreadable.other': "{n} éléments source illisibles — leurs copies ont été laissées intactes dans la destination",
    'history.confirm.clear.title': 'Effacer tout l’historique ?',
    'history.confirm.clear.body': 'Les entrées seront supprimées. Les dossiers de sauvegarde existants ne sont pas affectés.',
    'history.confirm.clear.action': 'Effacer',

    'statistics.backed_up': 'Sauvegardé',
    'statistics.tasks': 'Tâches',
    'statistics.successful_runs': 'Exécutions réussies',
    'statistics.aria.day': 'Sauvegardes du {day} : {bytes}',
    'statistics.aria.bars': 'Succès vs erreurs par tâche',
    'chart.empty.backups': 'Aucune sauvegarde',
    'chart.empty.tasks': 'Aucune tâche',
    'chart.legend.success': 'Succès',
    'chart.legend.error': 'Erreur',

    'settings.section.general': 'Général',
    'settings.section.options': 'Options de sauvegarde',
    'settings.section.filtering': 'Filtres',
    'settings.section.appearance': 'Apparence',
    'settings.section.language': 'Langue',
    'settings.section.background': 'Arrière-plan',
    'settings.section.updates': 'Mises à jour',
    'settings.section.diagnostics': 'Diagnostics',

    'settings.label.close_to_tray': 'Continuer en arrière-plan à la fermeture',
    'settings.label.autostart': 'Lancer driveby à l’ouverture de session',
    'settings.label.version': 'Version',
    'settings.label.check_updates_on_start': 'Vérifier les mises à jour au démarrage',
    'settings.tip.close_to_tray': "Garde driveby actif dans la zone de notification à la fermeture de la fenêtre, pour que les sauvegardes planifiées continuent de se déclencher.",
    'settings.tip.autostart': "Démarre driveby en arrière-plan à l’ouverture de session, pour que les sauvegardes planifiées s’exécutent sans ouvrir l’application.",
    'settings.toast.autostart_failed': 'Impossible de modifier le lancement au démarrage',

    'updates.up_to_date': 'driveby est à jour',
    'updates.available': 'La version {version} est disponible',
    'updates.installing': 'Téléchargement de la mise à jour — driveby redémarrera tout seul',
    'updates.action.installing': 'Installation…',
    'updates.action.check': 'Vérifier les mises à jour',
    'updates.action.checking': 'Vérification…',
    'updates.action.install': 'Installer et redémarrer',
    'updates.toast.available': 'Mise à jour disponible — voir Paramètres',
    'updates.toast.failed': 'Échec de la vérification : {error}',

    'settings.label.default_dest': 'Destination par défaut',
    'settings.dialog.default_dest': 'Sélectionner la destination par défaut',
    'settings.label.confirm_backup': 'Confirmer avant chaque sauvegarde',
    'settings.label.notifications': 'Notifications système',
    'settings.label.verify': 'Vérifier après copie',
    'settings.label.continue_on_error': 'Continuer en cas d’erreur',
    'settings.label.preserve_mtime': 'Préserver la date de modification',
    'settings.label.parallel_copies': 'Fichiers copiés simultanément',
    'settings.label.history_retention': 'Historique conservé',
    'settings.label.exclude': 'Motifs d’exclusion',
    'settings.label.appearance': 'Apparence',
    'settings.label.language': 'Langue',
    'settings.label.logs': 'Journaux d’application',

    'settings.tip.verify': "Relit chaque fichier copié et le compare à une empreinte calculée pendant l’écriture, pour détecter toute corruption.",
    'settings.tip.parallel_copies': "Nombre de fichiers copiés simultanément. 4 convient aux SSD et aux disques réseau ; choisissez 1 pour un disque mécanique.",
    'settings.tip.history_retention': "Ancienneté maximale de l’historique. Les exécutions plus anciennes sont supprimées ; les dossiers de sauvegarde ne sont jamais touchés.",
    'settings.tip.continue_on_error': "Saute les fichiers impossibles à copier — verrouillés ou sans droits — et poursuit au lieu d’arrêter toute la tâche.",
    'settings.tip.preserve_mtime': "Conserve la date de modification d’origine, ce qui permet aux exécutions suivantes de sauter instantanément les fichiers inchangés.",
    'settings.tip.exclude': "Un motif par ligne, ou séparés par des virgules. * remplace n’importe quels caractères, ** traverse les dossiers, ? un seul, et ! en début de ligne réinclut.",

    'settings.placeholder.exclude': '*.tmp\nnode_modules\n.DS_Store\n!important.tmp',

    'settings.retention.1d': '1 jour',
    'settings.retention.1w': '1 semaine',
    'settings.retention.1m': '1 mois',
    'settings.retention.1y': '1 an',

    'settings.theme.light': 'Clair',
    'settings.theme.dark': 'Sombre',
    'settings.theme.system': 'Système',

    'settings.toast.cannot_open_logs': 'Impossible d’ouvrir les journaux : {error}',
  },
};

const PLURAL_RULES = {};
function pluralRules(lang) {
  return (PLURAL_RULES[lang] ??= new Intl.PluralRules(lang));
}

export function translate(lang, key, params) {
  const dict = MESSAGES[lang] || MESSAGES[DEFAULT_LANGUAGE];
  const fallback = MESSAGES[DEFAULT_LANGUAGE];
  let s;
  if (params && typeof params.count === 'number') {
    const form = pluralRules(lang).select(params.count);
    s =
      dict[`${key}.${form}`] ??
      dict[`${key}.other`] ??
      fallback[`${key}.${form}`] ??
      fallback[`${key}.other`];
  }
  if (s === undefined) s = dict[key];
  if (s === undefined) s = fallback[key];
  if (s === undefined) return key;
  if (params) {
    for (const k of Object.keys(params)) {
      s = s.replace(new RegExp(`\\{${k}\\}`, 'g'), String(params[k]));
    }
  }
  return s;
}
