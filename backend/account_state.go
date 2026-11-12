package main

import (
	"context"
	"database/sql"
	"sync"
	"sync/atomic"
	"time"

	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
)

// Account holds everything that belongs to one linked WhatsApp account: its
// database, its whatsmeow client and every cache and queue built on them. One
// backend runs several of these side by side (see account.go); nothing in
// here may be shared between accounts.
type Account struct {
	id       string
	dbPath   string
	ctx      context.Context
	cancel   context.CancelFunc
	commands chan queuedCommand
	closing  atomic.Bool

	autoDownloadSlots   chan struct{}
	autoDownloadMu      sync.Mutex
	autoDownloadPending int
	avatarQueue         struct {
		sync.Mutex
		pending []string
		queued  map[string]bool
		running bool
	}
	blocklistMu                sync.Mutex
	blocklistSyncMu            sync.Mutex
	blocklistSyncedFor         *whatsmeow.Client
	blocklistDHash             string
	chatBroadcastTimer         *time.Timer
	chatBroadcastPendingSince  time.Time
	businessNameLookups        sync.Map
	joinableCallsMu            sync.Mutex
	joinableCalls              []map[string]interface{}
	liveCallsMu                sync.Mutex
	liveCalls                  map[string]*liveCall
	callClient                 *meowcaller.Client
	activeCalls                map[string]*meowcaller.Call
	activeCallsMutex           sync.Mutex
	activeVideoBridges         map[string]*VideoBridge
	activeVideoBridgesMutex    sync.Mutex
	activeScreenBridges        map[string]*VideoBridge
	activeAudioBridges         map[string]*UDPBridge
	activeAudioBridgesMutex    sync.Mutex
	incomingGroupCallJIDs      map[string]types.JID
	incomingGroupCallJIDsMutex sync.Mutex
	localScreenShares          map[string]*localScreenShare
	localScreenSharesMutex     sync.Mutex
	listsMu                    sync.Mutex
	favoritesSendMu            sync.Mutex
	displayNames               struct {
		sync.Mutex
		names   map[string]string
		search  map[string]chatSearchKeys
		expires time.Time
	}
	knownBusinesses        sync.Map
	serverFlagsMu          sync.RWMutex
	serverFlags            whatsmeow.ABProps
	appStateSendMu         sync.Mutex
	emptyChatHistoryAsked  sync.Map
	onDemandWaiters        sync.Map
	historyRequestMu       sync.Mutex
	lastHistoryRequest     time.Time
	channelFetchMu         sync.Mutex
	channelFetchRunning    map[string]bool
	channelFreshUntil      map[string]time.Time
	joinedGroupsMu         sync.RWMutex
	joinedGroups           map[string]bool
	subgroupParents        sync.Map
	convertStoredVotesOnce sync.Once
	pendingGroupDeletes    sync.Map
	sentGroupHistory       sync.Map
	groupListMu            sync.Mutex
	groupListCached        []*types.GroupInfo
	groupListFetched       time.Time
	followedChannelsMu     sync.Mutex
	followedChannelsLast   time.Time
	syncedAppPrefs         map[string]string
	activeDownloadsMu      sync.Mutex
	activeDownloads        map[string]context.CancelFunc
	autoDownloadFailed     map[string]bool
	autoDownloadFailedOnce sync.Once
	activeUploadsMu        sync.Mutex
	activeUploads          map[string]context.CancelFunc
	retryWaitersMu         sync.Mutex
	retryWaiters           map[string]chan bool
	pendingAutoOpens       sync.Map
	desiredPresenceMu      sync.Mutex
	desiredPresence        types.Presence
	sentPresence           types.Presence
	readPresenceTimer      *time.Timer
	presenceSubscribed     sync.Map
	stickerStore           struct {
		sync.Mutex
		packs []types.StickerPack
		at    time.Time
		data  map[string]stickerPackFetch
	}
	retryMu        sync.Mutex
	sendInFlight   map[string]bool
	retryAttempts  map[string]int
	retryNextAt    map[string]time.Time
	pairingAttempt struct {
		sync.Mutex
		cancel context.CancelFunc
	}
	pendingAlbumsMu          sync.Mutex
	pendingAlbums            map[string]*pendingAlbum
	groupSizesMu             sync.Mutex
	groupSizes               map[types.JID]int
	historySyncMutex         sync.Mutex
	historySyncChatsDone     int
	historySyncMsgsDone      int
	historySyncRefreshMu     sync.Mutex
	historySyncRefreshTimer  *time.Timer
	historySyncCheckpointMu  sync.Mutex
	historySyncCheckpointGen int
	historySyncPending       atomic.Int32
	populateChatsMu          sync.Mutex
	populateChatsRunning     bool
	populateChatsPending     bool
	appStateResyncMu         sync.Mutex
	lastAppStateResync       map[appstate.WAPatchName]time.Time
	appStateResyncCount      map[appstate.WAPatchName]int
	lastFatalRecoveryRequest map[appstate.WAPatchName]time.Time
	connRequests             chan connRequest
	connFailures             atomic.Int32
	connProbing              atomic.Bool
	connStateMu              sync.Mutex
	connStateCur             map[string]interface{}
	groupMetadataQueue       struct {
		sync.Mutex
		queued  []types.JID
		pending map[types.JID]bool
		tried   map[types.JID]time.Time
		running bool
	}
	recentSync struct {
		sync.Mutex
		active  bool
		percent int
		paused  bool
		timer   *time.Timer
	}
	historyAccessNoticeSent atomic.Bool
	lidMergeQueue           struct {
		sync.Mutex
		pending    map[string]types.JID
		renumbered map[string]renumber
		timer      *time.Timer
		since      time.Time
	}
	lidMergeMu            sync.Mutex
	container             *sqlstore.Container
	db                    *sql.DB
	pairingCode           string
	waClient              atomic.Pointer[whatsmeow.Client]
	mediaDirOnce          sync.Once
	mediaDirBase          string
	pendingResendsMu      sync.Mutex
	pendingResends        []resendTarget
	recheckChatLIDsOnce   sync.Once
	learnKeepArchivedOnce sync.Once
}

// initAccountState gives the fields their starting values.
func (acc *Account) initAccountState() {
	acc.autoDownloadSlots = make(chan struct{}, autoDownloadParallel)
	acc.avatarQueue = struct {
		sync.Mutex
		pending []string
		queued  map[string]bool
		running bool
	}{queued: map[string]bool{}}
	acc.liveCalls = map[string]*liveCall{}
	acc.activeCalls = make(map[string]*meowcaller.Call)
	acc.activeVideoBridges = make(map[string]*VideoBridge)
	acc.activeScreenBridges = make(map[string]*VideoBridge)
	acc.activeAudioBridges = make(map[string]*UDPBridge)
	acc.incomingGroupCallJIDs = make(map[string]types.JID)
	acc.localScreenShares = make(map[string]*localScreenShare)
	acc.channelFetchRunning = map[string]bool{}
	acc.channelFreshUntil = map[string]time.Time{}
	acc.syncedAppPrefs = map[string]string{
		"isPhotosAutodownloadEnabled":    "auto_download_photos",
		"isAudiosAutodownloadEnabled":    "auto_download_audio",
		"isVideosAutodownloadEnabled":    "auto_download_videos",
		"isDocumentsAutodownloadEnabled": "auto_download_documents",
		"mediaUploadQuality":             "hd_media",
		"mediaAutoDownloadQuality":       "hd_auto_download",
	}
	acc.activeDownloads = map[string]context.CancelFunc{}
	acc.autoDownloadFailed = map[string]bool{}
	acc.activeUploads = map[string]context.CancelFunc{}
	acc.retryWaiters = map[string]chan bool{}
	acc.desiredPresence = types.PresenceUnavailable
	acc.sendInFlight = map[string]bool{}
	acc.retryAttempts = map[string]int{}
	acc.retryNextAt = map[string]time.Time{}
	acc.pendingAlbums = map[string]*pendingAlbum{}
	acc.groupSizes = map[types.JID]int{}
	acc.lastAppStateResync = make(map[appstate.WAPatchName]time.Time)
	acc.appStateResyncCount = make(map[appstate.WAPatchName]int)
	acc.lastFatalRecoveryRequest = make(map[appstate.WAPatchName]time.Time)
	acc.connRequests = make(chan connRequest, 8)
	acc.connStateCur = map[string]interface{}{"state": connStateConnecting}
}
