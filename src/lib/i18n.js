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

    'backup.phase.syncing': 'Comparing',
    'backup.phase.copying': 'Copying',
    'backup.phase.pruning': 'Cleaning up',
    'backup.phase.verifying_icons': 'Checking folder icons',
    'backup.phase.finishing': 'Finishing',
    'backup.phase.verifying': 'Verifying',

    'restore.dialog.select': 'Select restore destination',
    'restore.dialog.title': 'Restore this backup?',
    'restore.dialog.body': 'Files will be written to:\n{destination}',
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
    'settings.tip.close_to_tray': 'Closing the window normally quits driveby, which also stops scheduled backups from running. With this on, driveby keeps running in the notification area instead — scheduled backups still fire, and you can reopen the window from the tray icon.',
    'settings.tip.autostart': 'Registers driveby with your system so it starts in the background when you log in. Combined with the setting above, scheduled backups run without you having to open the app.',
    'settings.toast.autostart_failed': 'Could not change the startup setting',

    'updates.up_to_date': 'driveby is up to date',
    'updates.available': 'Version {version} is available',
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
    'settings.label.history_limit': 'History entries kept',
    'settings.label.exclude': 'Exclude patterns',
    'settings.label.appearance': 'Appearance',
    'settings.label.language': 'Language',
    'settings.label.logs': 'Application logs',

    'settings.tip.verify': 'After copying, driveby reads each copied file back from the destination and compares it against a fingerprint taken while it was being written, to make sure nothing got corrupted on the way. Files skipped as unchanged were verified by the run that copied them.',
    'settings.tip.parallel_copies': 'How many files driveby copies at the same time. 4 is a good default for SSDs and network drives. Set it to 1 for an older spinning hard disk, where copying several files at once can be slower rather than faster.',
    'settings.tip.history_limit': 'How many past runs to keep in the history list. Older entries are dropped automatically. Existing backup folders are never affected.',
    'settings.tip.continue_on_error': "If a single file can't be copied — for example because it's locked by another app or you don't have permission — driveby will skip it and keep backing up everything else instead of stopping the whole job.",
    'settings.tip.preserve_mtime': "Keeps each file's original 'last modified' date when it's copied to the destination. This lets later backups instantly skip files that haven't changed, making repeat runs much faster.",
    'settings.tip.exclude': "List the files or folders you don't want backed up — one per line, or separated by commas. Use * to match any characters in a name, ** to match across folders, and ? for a single character. Start a line with ! to bring something back in (for example, !important.tmp keeps that file even if *.tmp is excluded).",

    'settings.placeholder.exclude': '*.tmp\nnode_modules\n.DS_Store\n!important.tmp',

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

    'backup.phase.syncing': 'Comparaison',
    'backup.phase.copying': 'Copie',
    'backup.phase.pruning': 'Nettoyage',
    'backup.phase.verifying_icons': 'Vérification des icônes',
    'backup.phase.finishing': 'Finalisation',
    'backup.phase.verifying': 'Vérification',

    'restore.dialog.select': 'Sélectionner la destination de restauration',
    'restore.dialog.title': 'Restaurer cette sauvegarde ?',
    'restore.dialog.body': 'Les fichiers seront écrits dans :\n{destination}',
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
    'settings.tip.close_to_tray': "Fermer la fenêtre quitte normalement driveby, ce qui empêche aussi les sauvegardes planifiées de s’exécuter. Avec cette option, driveby continue de tourner dans la zone de notification : les sauvegardes planifiées se déclenchent toujours et vous pouvez rouvrir la fenêtre depuis l’icône.",
    'settings.tip.autostart': "Inscrit driveby auprès de votre système pour qu’il démarre en arrière-plan à l’ouverture de session. Combiné à l’option ci-dessus, les sauvegardes planifiées s’exécutent sans que vous ayez à ouvrir l’application.",
    'settings.toast.autostart_failed': 'Impossible de modifier le lancement au démarrage',

    'updates.up_to_date': 'driveby est à jour',
    'updates.available': 'La version {version} est disponible',
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
    'settings.label.history_limit': 'Entrées d’historique conservées',
    'settings.label.exclude': 'Motifs d’exclusion',
    'settings.label.appearance': 'Apparence',
    'settings.label.language': 'Langue',
    'settings.label.logs': 'Journaux d’application',

    'settings.tip.verify': "Après la copie, driveby relit chaque fichier copié depuis la destination et le compare à une empreinte calculée pendant l’écriture, pour s’assurer qu’aucune corruption n’est survenue. Les fichiers inchangés ont été vérifiés lors de la sauvegarde qui les a copiés.",
    'settings.tip.parallel_copies': "Nombre de fichiers copiés en même temps. 4 convient bien aux SSD et aux disques réseau. Choisissez 1 pour un disque dur mécanique, où copier plusieurs fichiers à la fois peut ralentir plutôt qu’accélérer.",
    'settings.tip.history_limit': "Nombre d’exécutions passées conservées dans l’historique. Les plus anciennes sont supprimées automatiquement. Les dossiers de sauvegarde existants ne sont jamais affectés.",
    'settings.tip.continue_on_error': "Si un fichier ne peut pas être copié — par exemple parce qu’il est verrouillé par une autre application ou que vous n’avez pas les droits — driveby le saute et continue avec les autres au lieu d’arrêter toute la tâche.",
    'settings.tip.preserve_mtime': "Conserve la date de « dernière modification » d’origine de chaque fichier copié vers la destination. Cela permet aux sauvegardes suivantes de sauter immédiatement les fichiers inchangés, accélérant nettement les exécutions répétées.",
    'settings.tip.exclude': "Listez les fichiers ou dossiers à ne pas sauvegarder — un par ligne ou séparés par des virgules. Utilisez * pour n’importe quels caractères dans un nom, ** pour traverser les dossiers, et ? pour un seul caractère. Commencez une ligne par ! pour réinclure un élément (par exemple, !important.tmp conserve ce fichier même si *.tmp est exclu).",

    'settings.placeholder.exclude': '*.tmp\nnode_modules\n.DS_Store\n!important.tmp',

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
