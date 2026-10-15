; *** Inno Setup version 6.5.0+ Greek messages ***
;
; Encoding: UTF-8
;
; Note: When translating this text, do not add periods (.) to the end of
; messages that didn't have them already, because on those messages Inno
; Setup adds the periods automatically (appending a period would result in
; two periods being displayed).

[LangOptions]
; The following three entries are very important. Be sure to read and
; understand the '[LangOptions] section' topic in the help file.
LanguageName=Ελληνικά
LanguageID=$0408
LanguageCodePage=1253
; If the language you are translating to requires special font faces or
; sizes, uncomment any of the following entries and change them accordingly.
;DialogFontName=
;DialogFontSize=9
;DialogFontBaseScaleWidth=7
;DialogFontBaseScaleHeight=15
;WelcomeFontName=Segoe UI
;WelcomeFontSize=14

[Messages]

; *** Application titles
SetupAppTitle=Εγκατάσταση
SetupWindowTitle=Εγκατάσταση — %1
UninstallAppTitle=Απεγκατάσταση
UninstallAppFullTitle=Απεγκατάσταση — %1

; *** Misc. common
InformationTitle=Πληροφορίες
ConfirmTitle=Επιβεβαίωση
ErrorTitle=Σφάλμα

; *** SetupLdr messages
SetupLdrStartupMessage=Αυτό το πρόγραμμα θα εγκαταστήσει το %1 στον υπολογιστή σας. Να συνεχιστεί;
LdrCannotCreateTemp=Δεν είναι δυνατή η δημιουργία προσωρινού αρχείου. Η εγκατάσταση διακόπηκε
LdrCannotExecTemp=Δεν είναι δυνατή η εκτέλεση αρχείου στον προσωρινό κατάλογο. Η εγκατάσταση διακόπηκε
HelpTextNote=

; *** Startup error messages
LastErrorMessage=%1.%n%nΣφάλμα %2: %3
SetupFileMissing=Το αρχείο %1 λείπει από τον φάκελο εγκατάστασης. Παρακαλούμε διορθώστε το πρόβλημα ή αποκτήστε μια νέα έκδοση του προγράμματος.
SetupFileCorrupt=Τα αρχεία εγκατάστασης είναι κατεστραμμένα. Παρακαλούμε αποκτήστε ένα νέο αντίγραφο του προγράμματος.
SetupFileCorruptOrWrongVer=Τα αρχεία εγκατάστασης είναι κατεστραμμένα ή μη συμβατά με αυτή την έκδοση του προγράμματος εγκατάστασης. Παρακαλούμε διορθώστε το πρόβλημα ή αποκτήστε ένα νέο αντίγραφο του προγράμματος.
InvalidParameter=Η γραμμή εντολών περιέχει μη έγκυρη παράμετρο:%n%n%1
SetupAlreadyRunning=Το πρόγραμμα εγκατάστασης εκτελείται ήδη.
WindowsVersionNotSupported=Αυτό το πρόγραμμα δεν υποστηρίζει την έκδοση των Windows που είναι εγκατεστημένη σε αυτόν τον υπολογιστή.
WindowsServicePackRequired=Αυτό το πρόγραμμα απαιτεί %1 Service Pack %2 ή νεότερη έκδοση.
NotOnThisPlatform=Αυτό το πρόγραμμα δεν θα λειτουργήσει σε %1.
OnlyOnThisPlatform=Αυτό το πρόγραμμα μπορεί να εκτελεστεί μόνο σε %1.
OnlyOnTheseArchitectures=Η εγκατάσταση αυτού του προγράμματος είναι δυνατή μόνο σε εκδόσεις Windows για τις ακόλουθες αρχιτεκτονικές επεξεργαστές:%n%n%1
WinVersionTooLowError=Αυτό το πρόγραμμα απαιτεί %1 έκδοση %2 ή νεότερη.
WinVersionTooHighError=Το πρόγραμμα δεν μπορεί να εγκατασταθεί σε %1 έκδοση %2 ή νεότερη.
AdminPrivilegesRequired=Για να εγκαταστήσετε αυτό το πρόγραμμα, πρέπει να συνδεθείτε ως Διαχειριστής.
PowerUserPrivilegesRequired=Για να εγκαταστήσετε αυτό το πρόγραμμα, πρέπει να συνδεθείτε ως Διαχειριστής ή ως μέλος της ομάδας «Έμπειροι χρήστες» (Power Users).
SetupAppRunningError=Εντοπίστηκε εκτελούμενη εγκατάσταση του %1.%n%nΠαρακαλούμε κλείστε όλες τις εκτελούμενες εγγραφές της εφαρμογής και μετά πατήστε «OK» για συνέχεια ή «Ακύρωση» για έξοδο.
UninstallAppRunningError=Ο απεγκαταστάτης εντόπισε εκτελούμενη εγκατάσταση του %1.%n%nΠαρακαλούμε κλείστε όλες τις εκτελούμενες εγγραφές της εφαρμογής και μετά πατήστε «OK» για συνέχεια ή «Ακύρωση» για έξοδο.

; *** Startup questions
PrivilegesRequiredOverrideTitle=Επιλογή τρόπου εγκατάστασης
PrivilegesRequiredOverrideInstruction=Επιλέξτε τρόπο εγκατάστασης
PrivilegesRequiredOverrideText1=Το %1 μπορεί να εγκατασταθεί για όλους τους χρήστες (απαιτούνται προνόμια διαχειριστή) ή μόνο για εσάς.
PrivilegesRequiredOverrideText2=Το %1 μπορεί να εγκατασταθεί μόνο για εσάς ή για όλους τους χρήστες (απαιτούνται προνόμια διαχειριστή).
PrivilegesRequiredOverrideAllUsers=Εγκατάσταση για &όλους τους χρήστες
PrivilegesRequiredOverrideAllUsersRecommended=Εγκατάσταση για &όλους τους χρήστες (συνιστάται)
PrivilegesRequiredOverrideCurrentUser=Εγκατάσταση μόνο για &μένα
PrivilegesRequiredOverrideCurrentUserRecommended=Εγκατάσταση μόνο για &μένα (συνιστάται)

; *** Misc. errors
ErrorCreatingDir=Δεν είναι δυνατή η δημιουργία φακέλου "%1"
ErrorTooManyFilesInDir=Δεν είναι δυνατή η δημιουργία αρχείου στον κατάλογο "%1", επειδή περιέχει πάρα πολλά αρχεία

; *** Setup common messages
ExitSetupTitle=Έξοδος από το πρόγραμμα εγκατάστασης
ExitSetupMessage=Η εγκατάσταση δεν ολοκληρώθηκε. Αν γίνει τώρα έξοδος, το πρόγραμμα δεν θα εγκατασταθεί.%n%nΘα μπορέσετε να ολοκληρώσετε την εγκατάσταση εκτελώντας το πρόγραμμα εγκατάστασης αργότερα.%n%nΝα γίνει έξοδος από το πρόγραμμα εγκατάστασης;
AboutSetupMenuItem=&Σχετικά...
AboutSetupTitle=Σχετικά με την εγκατάσταση
AboutSetupMessage=%1, έκδοση %2%n%3%n%nΙστότοπος του %1:%n%4
AboutSetupNote=
TranslatorNote=Ελληνική γλώσσα

; *** Buttons
ButtonBack=< &Πίσω
ButtonNext=&Επόμενο >
ButtonInstall=&Εγκατάσταση
ButtonOK=OK
ButtonCancel=Ακύρωση
ButtonYes=&Ναι
ButtonYesToAll=Ναι για &όλα
ButtonNo=&Όχι
ButtonNoToAll=Ό&χι για όλα
ButtonFinish=&Τέλος
ButtonBrowse=&Αναζήτηση...
ButtonWizardBrowse=&Αναζήτηση...
ButtonNewFolder=&Δημιουργία φακέλου

; *** "Select Language" dialog messages
SelectLanguageTitle=Επιλέξτε γλώσσα εγκατάστασης
SelectLanguageLabel=Επιλέξτε τη γλώσσα που θα χρησιμοποιηθεί κατά τη διάρκεια της εγκατάστασης.

; *** Common wizard text
ClickNext=Πατήστε «Επόμενο» για συνέχεια ή «Ακύρωση» για έξοδο από το πρόγραμμα εγκατάστασης.
BeveledLabel=
BrowseDialogTitle=Αναζήτηση φακέλων
BrowseDialogLabel=Επιλέξτε φάκελο από τη λίστα και πατήστε «OK».
NewFolderName=Νέος φάκελος

; *** "Welcome" wizard page
WelcomeLabel1=Καλώς ήλθατε στον Οδηγό εγκατάστασης του [name]
WelcomeLabel2=Το πρόγραμμα θα εγκαταστήσει το [name/ver] στον υπολογιστή σας.%n%nΣυνιστάται να κλείσετε όλες τις άλλες εφαρμογές πριν συνεχίσετε.

; *** "Password" wizard page
WizardPassword=Κωδικός πρόσβασης
PasswordLabel1=Αυτό το πρόγραμμα προστατεύεται με κωδικό πρόσβασης.
PasswordLabel3=Παρακαλούμε πληκτρολογήστε τον κωδικό πρόσβασης και πατήστε «Επόμενο». Οι κωδικοί πρόσβασης πρέπει να εισάγονται με διάκριση πεζών/κεφαλαίων.
PasswordEditLabel=&Κωδικός πρόσβασης:
IncorrectPassword=Ο κωδικός πρόσβασης που εισάγατε είναι λανθασμένος. Παρακαλούμε δοκιμάστε ξανά.

; *** "License Agreement" wizard page
WizardLicense=Άδεια χρήσης
LicenseLabel=Παρακαλούμε διαβάστε τις ακόλουθες σημαντικές πληροφορίες πριν συνεχίσετε.
LicenseLabel3=Παρακαλούμε διαβάστε την παρακάτω άδεια χρήσης. Πρέπει να αποδεχτείτε τους όρους της πριν συνεχίσετε.
LicenseAccepted=Απο&δέχομαι τους όρους της άδειας
LicenseNotAccepted=Δεν απο&δέχομαι τους όρους της άδειας

; *** "Information" wizard pages
WizardInfoBefore=Πληροφορίες
InfoBeforeLabel=Παρακαλούμε διαβάστε τις ακόλουθες σημαντικές πληροφορίες πριν συνεχίσετε.
InfoBeforeClickLabel=Όταν είστε έτοιμοι να συνεχίσετε την εγκατάσταση, πατήστε «Επόμενο».
WizardInfoAfter=Πληροφορίες
InfoAfterLabel=Παρακαλούμε διαβάστε τις ακόλουθες σημαντικές πληροφορίες πριν συνεχίσετε.
InfoAfterClickLabel=Όταν είστε έτοιμοι να συνεχίσετε την εγκατάσταση, πατήστε «Επόμενο».

; *** "User Information" wizard page
WizardUserInfo=Πληροφορίες χρήστη
UserInfoDesc=Παρακαλούμε εισάγετε τα στοιχεία σας.
UserInfoName=&Ονοματεπώνυμο χρήστη:
UserInfoOrg=&Οργανισμός:
UserInfoSerial=&Σειριακός αριθμός:
UserInfoNameRequired=Πρέπει να εισάγετε ένα όνομα.

; *** "Select Destination Location" wizard page
WizardSelectDir=Επιλογή φακέλου εγκατάστασης
SelectDirDesc=Σε ποιον φάκελο θέλετε να εγκατασταθεί το [name];
SelectDirLabel3=Το πρόγραμμα θα εγκαταστήσει το [name] στον παρακάτω φάκελο.
SelectDirBrowseLabel=Για συνέχεια, πατήστε «Επόμενο». Αν θέλετε να επιλέξετε άλλο φάκελο, πατήστε «Αναζήτηση».
DiskSpaceGBLabel=Απαιτούνται τουλάχιστον [gb] GB ελεύθερου χώρου στον δίσκο.
DiskSpaceMBLabel=Απαιτούνται τουλάχιστον [mb] MB ελεύθερου χώρου στον δίσκο.
CannotInstallToNetworkDrive=Η εγκατάσταση δεν μπορεί να γίνει σε δικτυακό δίσκο.
CannotInstallToUNCPath=Η εγκατάσταση δεν μπορεί να γίνει σε φάκελο διαδρομής UNC.
InvalidPath=Πρέπει να ορίσετε πλήρη διαδρομή με γράμμα δίσκου, για παράδειγμα:%n%nC:\APP%n%nή σε μορφή UNC:%n%n\\όνομα_διακομιστή\κοινόχρηστος_φάκελος
InvalidDrive=Ο δίσκος ή η δικτυακή διαδρομή που επιλέξατε δεν υπάρχει ή δεν είναι προσβάσιμος. Παρακαλούμε επιλέξτε άλλον.
DiskSpaceWarningTitle=Δεν υπάρχει αρκετός χώρος στον δίσκο
DiskSpaceWarning=Η εγκατάσταση απαιτεί τουλάχιστον %1 KB ελεύθερου χώρου, ενώ στον επιλεγμένο δίσκο είναι διαθέσιμοι μόνο %2 KB.%n%nΘέλετε ωστόσο να συνεχίσετε την εγκατάσταση;
DirNameTooLong=Το όνομα ή η διαδρομή του φακέλου υπερβαίνει το επιτρεπόμενο μήκος.
InvalidDirName=Το ορισμένο όνομα φακέλου είναι μη έγκυρο.
BadDirName32=Το όνομα του φακέλου δεν μπορεί να περιέχει τους χαρακτήρες:%n%n%1
DirExistsTitle=Ο φάκελος υπάρχει
DirExists=Ο φάκελος%n%n%1%n%nυπάρχει ήδη. Να γίνει ούτως ή άλλως η εγκατάσταση σε αυτόν τον φάκελο;
DirDoesntExistTitle=Ο φάκελος δεν υπάρχει
DirDoesntExist=Ο φάκελος%n%n%1%n%nδεν υπάρχει. Θέλετε να τον δημιουργήσετε;

; *** "Select Components" wizard page
WizardSelectComponents=Επιλογή στοιχείων
SelectComponentsDesc=Ποια στοιχεία πρέπει να εγκατασταθούν;
SelectComponentsLabel2=Επιλέξτε τα στοιχεία που θέλετε να εγκατασταθούν· αποεπιλέξτε τα στοιχεία που δεν χρειάζεται να εγκατασταθούν. Πατήστε «Επόμενο» όταν είστε έτοιμοι να συνεχίσετε.
FullInstallation=Πλήρης εγκατάσταση
; if possible don't translate 'Compact' as 'Minimal' (I mean 'Minimal' in your language)
CompactInstallation=Συμπαγής εγκατάσταση
CustomInstallation=Προσαρμοσμένη εγκατάσταση
NoUninstallWarningTitle=Εγκατεστημένα στοιχεία
NoUninstallWarning=Το πρόγραμμα εγκατάστασης εντόπισε ότι τα παρακάτω στοιχεία είναι ήδη εγκατεστημένα στον υπολογιστή σας:%n%n%1%n%nΗ αποεπιλογή αυτών των στοιχείων δεν θα τα διαγράψει.%n%nΣυνέχεια;
ComponentSize1=%1 KB
ComponentSize2=%1 MB
ComponentsDiskSpaceGBLabel=Η τρέχουσα επιλογή απαιτεί τουλάχιστον [gb] GB στον δίσκο.
ComponentsDiskSpaceMBLabel=Η τρέχουσα επιλογή απαιτεί τουλάχιστον [mb] MB στον δίσκο.

; *** "Select Additional Tasks" wizard page
WizardSelectTasks=Επιλέξτε πρόσθετες εργασίες
SelectTasksDesc=Ποιες πρόσθετες εργασίες πρέπει να εκτελεστούν;
SelectTasksLabel2=Επιλέξτε τις πρόσθετες εργασίες που πρέπει να εκτελεστούν κατά την εγκατάσταση του [name] και μετά πατήστε «Επόμενο»:

; *** "Select Start Menu Folder" wizard page
WizardSelectProgramGroup=Επιλέξτε φάκελο στο μενού «Έναρξη»
SelectStartMenuFolderDesc=Σε ποιον φάκελο πρέπει το πρόγραμμα εγκατάστασης να δημιουργήσει συντομεύσεις;
SelectStartMenuFolderLabel3=Το πρόγραμμα θα δημιουργήσει τις συντομεύσεις του προγράμματος στον παρακάτω φάκελο του μενού «Έναρξη».
SelectStartMenuFolderBrowseLabel=Για συνέχεια, πατήστε «Επόμενο». Αν θέλετε να επιλέξετε άλλο φάκελο, πατήστε «Αναζήτηση».
MustEnterGroupName=Πρέπει να εισάγετε ένα όνομα φακέλου.
GroupNameTooLong=Το όνομα ή η διαδρομή του φακέλου της ομάδας υπερβαίνει το επιτρεπόμενο μήκος.
InvalidGroupName=Το ορισμένο όνομα φακέλου είναι μη έγκυρο.
BadGroupName=Το όνομα του φακέλου δεν μπορεί να περιέχει τους χαρακτήρες:%n%n%1
NoProgramGroupCheck2=&Να μην δημιουργηθεί φάκελος στο μενού «Έναρξη»

; *** "Ready to Install" wizard page
WizardReady=Η εγκατάσταση είναι έτοιμη να ξεκινήσει
ReadyLabel1=Το πρόγραμμα εγκατάστασης είναι έτοιμο να ξεκινήσει την εγκατάσταση του [name] στον υπολογιστή σας.
ReadyLabel2a=Πατήστε «Εγκατάσταση» για συνέχεια ή «Πίσω» αν θέλετε να εξετάσετε ή να αλλάξετε τις ρυθμίσεις εγκατάστασης.
ReadyLabel2b=Πατήστε «Εγκατάσταση» για συνέχεια.
ReadyMemoUserInfo=Πληροφορίες χρήστη:
ReadyMemoDir=Φάκελος εγκατάστασης:
ReadyMemoType=Τύπος εγκατάστασης:
ReadyMemoComponents=Επιλεγμένα στοιχεία:
ReadyMemoGroup=Φάκελος στο μενού «Έναρξη»:
ReadyMemoTasks=Πρόσθετες εργασίες:

; *** TDownloadWizardPage wizard page and DownloadTemporaryFile
DownloadingLabel2=Γίνεται λήψη αρχείων...
ButtonStopDownload=&Διακοπή λήψης
StopDownload=Θέλετε πράγματι να διακόψετε τη λήψη;
ErrorDownloadAborted=Η λήψη διακόπηκε
ErrorDownloadFailed=Σφάλμα λήψης: %1 %2
ErrorDownloadSizeFailed=Σφάλμα λήψης μεγέθους: %1 %2
ErrorProgress=Σφάλμα προόδου: %1 από %2
ErrorFileSize=Μη έγκυρο μέγεθος αρχείου: αναμενόταν %1, λήφθηκε %2

; *** TExtractionWizardPage wizard page and Extract7ZipArchive
ExtractingLabel=Γίνεται αποσυμπίεση αρχείων...
ButtonStopExtraction=&Διακοπή αποσυμπίεσης
StopExtraction=Είστε βέβαιοι ότι θέλετε να διακόψετε την αποσυμπίεση;
ErrorExtractionAborted=Η αποσυμπίεση διακόπηκε
ErrorExtractionFailed=Σφάλμα αποσυμπίεσης: %1

; *** Archive extraction failure details
ArchiveIncorrectPassword=Λανθασμένος κωδικός πρόσβασης
ArchiveIsCorrupted=Το συμπιεσμένο αρχείο είναι κατεστραμμένο
ArchiveUnsupportedFormat=Μη υποστηριζόμενη μορφή συμπιεσμένου αρχείου

; *** "Preparing to Install" wizard page
WizardPreparing=Προετοιμασία εγκατάστασης
PreparingDesc=Το πρόγραμμα εγκατάστασης προετοιμάζεται να εγκαταστήσει το [name] στον υπολογιστή σας.
PreviousInstallNotCompleted=Η εγκατάσταση ή η απεγκατάσταση της προηγούμενης εφαρμογής δεν ολοκληρώθηκε. Θα χρειαστεί να επανεκκινήσετε τον υπολογιστή για να ολοκληρωθεί αυτή η εγκατάσταση.%n%nΜετά την επανεκκίνηση εκτελέστε ξανά το πρόγραμμα εγκατάστασης για να ολοκληρωθεί η εγκατάσταση του [name].
CannotContinue=Δεν είναι δυνατή η συνέχιση της εγκατάστασης. Πατήστε «Ακύρωση» για έξοδο από την εγκατάσταση.
ApplicationsFound=Οι παρακάτω εφαρμογές χρησιμοποιούν αρχεία που το πρόγραμμα εγκατάστασης πρέπει να ενημερώσει. Συνιστάται να επιτρέψετε στο πρόγραμμα εγκατάστασης να τις κλείσει αυτόματα.
ApplicationsFound2=Οι παρακάτω εφαρμογές χρησιμοποιούν αρχεία που το πρόγραμμα εγκατάστασης πρέπει να ενημερώσει. Συνιστάται να επιτρέψετε στο πρόγραμμα εγκατάστασης να τις κλείσει αυτόματα. Με την ολοκλήρωση της εγκατάστασης, το πρόγραμμα εγκατάστασης θα επιχειρήσει να τις επανεκκινήσει.
CloseApplications=&Αυτόματα κλείσιμο των εφαρμογών
DontCloseApplications=&Να μη κλείσουν οι εφαρμογές
ErrorCloseApplications=Το πρόγραμμα εγκατάστασης δεν μπόρεσε να κλείσει αυτόματα όλες τις εφαρμογές. Πριν συνεχίσετε την εγκατάσταση, συνιστάται να κλείσετε όλες τις εφαρμογές που χρησιμοποιούν τα προς ενημέρωση αρχεία.
PrepareToInstallNeedsRestart=Το πρόγραμμα εγκατάστασης πρέπει να επανεκκινήσει τον υπολογιστή σας. Μετά την επανεκκίνηση, παρακαλούμε εκτελέστε ξανά το πρόγραμμα εγκατάστασης για να ολοκληρωθεί η εγκατάσταση του [name].%n%nΝα γίνει επανεκκίνηση τώρα;

; *** "Installing" wizard page
WizardInstalling=Εγκατάσταση
InstallingLabel=Παρακαλούμε περιμένετε μέχρι το [name] να εγκατασταθεί στον υπολογιστή σας.

; *** "Setup Completed" wizard page
FinishedHeadingLabel=Ολοκλήρωση του Οδηγού εγκατάστασης του [name]
FinishedLabelNoIcons=Το πρόγραμμα [name] εγκαταστάθηκε στον υπολογιστή σας.
FinishedLabel=Το πρόγραμμα [name] εγκαταστάθηκε στον υπολογιστή σας. Η εφαρμογή μπορεί να εκτελεστεί μέσω του αντίστοιχου εικονιδίου.
ClickFinish=Πατήστε «Τέλος» για έξοδο από το πρόγραμμα εγκατάστασης.
FinishedRestartLabel=Για την ολοκλήρωση της εγκατάστασης του [name] απαιτείται επανεκκίνηση του υπολογιστή. Να γίνει επανεκκίνηση τώρα;
FinishedRestartMessage=Για την ολοκλήρωση της εγκατάστασης του [name] απαιτείται επανεκκίνηση του υπολογιστή.%n%nΝα γίνει επανεκκίνηση τώρα;
ShowReadmeCheck=Θέλω να δω το αρχείο README
YesRadio=&Ναι, επανεκκίνηση του υπολογιστή τώρα
NoRadio=&Όχι, η επανεκκίνηση θα γίνει αργότερα
; used for example as 'Run MyProg.exe'
RunEntryExec=Εκτέλεση %1
; used for example as 'View Readme.txt'
RunEntryShellExec=Προβολή %1

; *** "Setup Needs the Next Disk" stuff
ChangeDiskTitle=Χρειάζεται η εισαγωγή του επόμενου δίσκου
SelectDiskLabel2=Παρακαλούμε εισάγετε τον δίσκο %1 και πατήστε «OK».%n%nΑν τα αρχεία αυτού του δίσκου βρίσκονται σε φάκελο διαφορετικό από τον εμφανιζόμενο παρακάτω, ορίστε τη σωστή διαδρομή ή πατήστε «Αναζήτηση».
PathLabel=&Διαδρομή:
FileNotInDir2=Το αρχείο "%1" δεν βρέθηκε στο "%2". Παρακαλούμε εισάγετε τον σωστό δίσκο ή επιλέξτε άλλο φάκελο.
SelectDirectoryLabel=Παρακαλούμε ορίστε τη διαδρομή του επόμενου δίσκου.

; *** Installation phase messages
SetupAborted=Η εγκατάσταση δεν ολοκληρώθηκε.%n%nΠαρακαλούμε διορθώστε το πρόβλημα και εκτελέστε την εγκατάσταση ξανά.
AbortRetryIgnoreSelectAction=Επιλέξτε ενέργεια
AbortRetryIgnoreRetry=Νέα &προσπάθεια
AbortRetryIgnoreIgnore=&Αγνόηση του σφάλματος και συνέχεια
AbortRetryIgnoreCancel=Ακύρωση εγκατάστασης
RetryCancelSelectAction=Επιλέξτε ενέργεια
RetryCancelRetry=Νέα &προσπάθεια
RetryCancelCancel=Ακύρωση

; *** Installation status messages
StatusClosingApplications=Κλείνουν οι εφαρμογές...
StatusCreateDirs=Δημιουργούνται φάκελοι...
StatusExtractFiles=Εξάγονται τα αρχεία...
StatusDownloadFiles=Γίνεται λήψη των αρχείων...
StatusCreateIcons=Δημιουργούνται οι συντομεύσεις του προγράμματος...
StatusCreateIniEntries=Δημιουργούνται αρχεία INI...
StatusCreateRegistryEntries=Δημιουργούνται εγγραφές μητρώου...
StatusRegisterFiles=Εγγράφονται τα αρχεία...
StatusSavingUninstall=Αποθηκεύονται οι πληροφορίες απεγκατάστασης...
StatusRunProgram=Ολοκληρώνεται η εγκατάσταση...
StatusRestartingApplications=Επανεκκινούνται οι εφαρμογές...
StatusRollback=Επαναφορά αλλαγών...

; *** Misc. errors
ErrorInternal2=Εσωτερικό σφάλμα: %1
ErrorFunctionFailedNoCode=%1: αποτυχία
ErrorFunctionFailed=%1: αποτυχία· κωδικός %2
ErrorFunctionFailedWithMessage=%1: αποτυχία· κωδικός %2.%n%3
ErrorExecutingProgram=Δεν είναι δυνατή η εκτέλεση του αρχείου:%n%1

; *** Registry errors
ErrorRegOpenKey=Σφάλμα ανοίγματος κλειδιού μητρώου:%n%1\%2
ErrorRegCreateKey=Σφάλμα δημιουργίας κλειδιού μητρώου:%n%1\%2
ErrorRegWriteKey=Σφάλμα εγγραφής στο κλειδί μητρώου:%n%1\%2

; *** INI errors
ErrorIniEntry=Σφάλμα δημιουργίας εγγραφής στο αρχείο INI "%1".

; *** File copying errors
FileAbortRetryIgnoreSkipNotRecommended=&Παράλειψη αυτού του αρχείου (δεν συνιστάται)
FileAbortRetryIgnoreIgnoreNotRecommended=&Αγνόηση του σφάλματος και συνέχεια (δεν συνιστάται)
SourceIsCorrupted=Το αρχείο πηγής είναι κατεστραμμένο
SourceDoesntExist=Το αρχείο πηγής "%1" δεν υπάρχει
SourceVerificationFailed=Ο έλεγχος του αρχείου πηγής απέτυχε: %1
VerificationSignatureDoesntExist=Το αρχείο υπογραφής "%1" δεν υπάρχει
VerificationSignatureInvalid=Το αρχείο υπογραφής "%1" δεν είναι έγκυρο
VerificationKeyNotFound=Το αρχείο υπογραφής "%1" χρησιμοποιεί άγνωστο κλειδί
VerificationFileNameIncorrect=Μη έγκυρο όνομα αρχείου
VerificationFileTagIncorrect=Μη έγκυρη ετικέτα αρχείου
VerificationFileSizeIncorrect=Μη έγκυρο μέγεθος αρχείου
VerificationFileHashIncorrect=Μη έγκυρο hash αρχείου
ExistingFileReadOnly2=Δεν είναι δυνατή η αντικατάσταση του υπάρχοντος αρχείου, επειδή είναι σημειωμένο ως «μόνο για ανάγνωση».
ExistingFileReadOnlyRetry=&Αφαίρεση του χαρακτηρισμού «μόνο για ανάγνωση» και νέα προσπάθεια
ExistingFileReadOnlyKeepExisting=&Διατήρηση του αρχείου στη θέση του
ErrorReadingExistingDest=Παρουσιάστηκε σφάλμα κατά την ανάγνωση του υπάρχοντος αρχείου:
FileExistsSelectAction=Επιλέξτε ενέργεια
FileExists2=Το αρχείο υπάρχει ήδη.
FileExistsOverwriteExisting=&Αντικατάσταση του υπάρχοντος αρχείου
FileExistsKeepExisting=&Διατήρηση του υπάρχοντος αρχείου
FileExistsOverwriteOrKeepAll=&Επανάληψη της ενέργειας για όλες τις επόμενες συγκρούσεις
ExistingFileNewerSelectAction=Επιλέξτε ενέργεια
ExistingFileNewer2=Το υπάρχον αρχείο είναι νεότερο από αυτό που εγκαθίσταται.
ExistingFileNewerOverwriteExisting=&Αντικατάσταση του υπάρχοντος αρχείου
ExistingFileNewerKeepExisting=&Διατήρηση του υπάρχοντος αρχείου (συνιστάται)
ExistingFileNewerOverwriteOrKeepAll=&Επανάληψη της ενέργειας για όλες τις επόμενες συγκρούσεις
ErrorChangingAttr=Παρουσιάστηκε σφάλμα κατά την αλλαγή των ιδιοτήτων του υπάρχοντος αρχείου:
ErrorCreatingTemp=Παρουσιάστηκε σφάλμα κατά τη δημιουργία αρχείου στον φάκελο προορισμού:
ErrorReadingSource=Παρουσιάστηκε σφάλμα κατά την ανάγνωση του αρχείου πηγής:
ErrorCopying=Παρουσιάστηκε σφάλμα κατά την αντιγραφή του αρχείου:
ErrorDownloading=Παρουσιάστηκε σφάλμα κατά τη λήψη του αρχείου:
ErrorExtracting=Παρουσιάστηκε σφάλμα κατά την αποσυμπίεση του αρχείου:
ErrorReplacingExistingFile=Παρουσιάστηκε σφάλμα κατά την αντικατάσταση του υπάρχοντος αρχείου:
ErrorRestartReplace=Σφάλμα επανεκκίνησης και αντικατάστασης:
ErrorRenamingTemp=Παρουσιάστηκε σφάλμα κατά τη μετονομασία του αρχείου στον φάκελο προορισμού:
ErrorRegisterServer=Δεν είναι δυνατή η εγγραφή της DLL/OCX: %1
ErrorRegSvr32Failed=Σφάλμα κατά την εκτέλεση του RegSvr32, κωδικός εξόδου %1
ErrorRegisterTypeLib=Δεν είναι δυνατή η εγγραφή της βιβλιοθήκης τύπων (Type Library): %1

; *** Uninstall display name markings
; used for example as 'My Program (32-bit)'
UninstallDisplayNameMark=%1 (%2)
; used for example as 'My Program (32-bit, All users)'
UninstallDisplayNameMarks=%1 (%2, %3)
UninstallDisplayNameMark32Bit=32 bit
UninstallDisplayNameMark64Bit=64 bit
UninstallDisplayNameMarkAllUsers=Όλοι οι χρήστες
UninstallDisplayNameMarkCurrentUser=Τρέχων χρήστης

; *** Post-installation errors
ErrorOpeningReadme=Παρουσιάστηκε σφάλμα κατά το άνοιγμα του αρχείου README.
ErrorRestartingComputer=Το πρόγραμμα εγκατάστασης δεν μπόρεσε να επανεκκινήσει τον υπολογιστή. Παρακαλούμε κάντε το μόνοι σας.

; *** Uninstaller messages
UninstallNotFound=Το αρχείο "%1" δεν υπάρχει· η απεγκατάσταση δεν είναι δυνατή.
UninstallOpenError=Δεν είναι δυνατό το άνοιγμα του αρχείου "%1". Η απεγκατάσταση δεν είναι δυνατή
UninstallUnsupportedVer=Το αρχείο καταγραφής απεγκατάστασης "%1" δεν αναγνωρίζεται από αυτή την έκδοση του προγράμματος απεγκατάστασης. Η απεγκατάσταση δεν είναι δυνατή
UninstallUnknownEntry=Συναντήθηκε άγνωστη εγγραφή (%1) στο αρχείο καταγραφής απεγκατάστασης
ConfirmUninstall=Είστε βέβαιοι ότι θέλετε να απεγκαταστήσετε το %1 και όλα τα στοιχεία του προγράμματος;
UninstallOnlyOnWin64=Η απεγκατάσταση αυτού του προγράμματος είναι δυνατή μόνο σε 64-bit Windows.
OnlyAdminCanUninstall=Αυτό το πρόγραμμα μπορεί να απεγκατασταθεί μόνο από χρήστη με προνόμια διαχειριστή.
UninstallStatusLabel=Παρακαλούμε περιμένετε μέχρι το %1 να αφαιρεθεί από τον υπολογιστή σας.
UninstalledAll=Το πρόγραμμα %1 αφαιρέθηκε πλήρως από τον υπολογιστή σας.
UninstalledMost=Η απεγκατάσταση του %1 ολοκληρώθηκε.%n%nΜερικά στοιχεία δεν μπόρεσαν να διαγραφούν. Μπορείτε να τα διαγράψετε μόνοι σας.
UninstalledAndNeedsRestart=Για την ολοκλήρωση της απεγκατάστασης του %1 πρέπει να επανεκκινηθεί ο υπολογιστής σας.%n%nΝα γίνει επανεκκίνηση τώρα;
UninstallDataCorrupted=Το αρχείο "%1" είναι κατεστραμμένο. Η απεγκατάσταση δεν είναι δυνατή

; *** Uninstallation phase messages
ConfirmDeleteSharedFileTitle=Διαγραφή κοινόχρηστου αρχείου;
ConfirmDeleteSharedFile2=Το σύστημα αναφέρει ότι το παρακάτω κοινόχρηστο αρχείο δεν χρησιμοποιείται πια από άλλες εφαρμογές. Επιβεβαιώνετε τη διαγραφή του αρχείου;%n%nΑν υπάρχουν προγράμματα που εξακολουθούν να το χρησιμοποιούν και διαγραφεί, δεν θα λειτουργούν σωστά. Αν δεν είστε σίγουροι, επιλέξτε «Όχι». Το αρχείο που θα παραμείνει δεν θα βλάψει το σύστημά σας.
SharedFileNameLabel=Όνομα αρχείου:
SharedFileLocationLabel=Θέση:
WizardUninstalling=Κατάσταση απεγκατάστασης
StatusUninstalling=Γίνεται απεγκατάσταση του %1...

; *** Shutdown block reasons
ShutdownBlockReasonInstallingApp=Εγκατάσταση του %1.
ShutdownBlockReasonUninstallingApp=Απεγκατάσταση του %1.

; The custom messages below aren't used by Setup itself, but if you make
; use of them in your scripts, you'll want to translate them.

[CustomMessages]
NameAndVersion=%1, έκδοση %2
AdditionalIcons=Επιπλέον εικονίδια:
CreateDesktopIcon=&Δημιουργία εικονιδίου στην Επιφάνεια εργασίας
CreateQuickLaunchIcon=&Δημιουργία εικονιδίου στη Γραμμή γρήγορης εκκίνησης
ProgramOnTheWeb=Ιστότοπος του %1 στο Διαδίκτυο
UninstallProgram=Απεγκατάσταση του %1
LaunchProgram=Εκτέλεση %1
AssocFileExtension=Σύνδε&ση του %1 με αρχεία επέκτασης %2
AssocingFileExtension=Σύνδεση του %1 με αρχεία %2...
AutoStartProgramGroupDescription=Αυτόματη εκκίνηση:
AutoStartProgram=Αυτόματη εκκίνηση του %1
AddonHostProgramNotFound=Το %1 δεν βρέθηκε στον φάκελο που ορίσατε.%n%nΘέλετε ωστόσο να συνεχίσετε;
