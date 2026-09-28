package main

import (
	"context"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// The chat-list refresh (populateChatsFromStore) re-reads the group list and
// the followed channels from WhatsApp's servers. It reruns after every
// app-state collection, contact batch and history chunk, which right after
// linking meant eight fetches of each in under 20 seconds - enough for the
// server to answer the channel query with "rate-overlimit". Within these
// windows the refresh reuses what it last fetched; anything that knows the
// group list changed (joining, creating, the server saying it's dirty) drops
// the cached copy first.
const (
	groupListMaxAge       = 30 * time.Second
	followedChannelsEvery = time.Minute
)

var (
	groupListMu      sync.Mutex
	groupListCached  []*types.GroupInfo
	groupListFetched time.Time

	followedChannelsMu   sync.Mutex
	followedChannelsLast time.Time
)

// serverGroupList is the server's group list, fetched at most once per
// groupListMaxAge unless invalidateJoinedGroups was called.
func serverGroupList() ([]*types.GroupInfo, error) {
	groupListMu.Lock()
	defer groupListMu.Unlock()
	if groupListCached != nil && time.Since(groupListFetched) < groupListMaxAge {
		return groupListCached, nil
	}
	groups, err := wa().GetJoinedGroups(context.Background())
	if err != nil {
		return nil, err
	}
	groupListCached, groupListFetched = groups, time.Now()
	return groups, nil
}

// invalidateJoinedGroups makes the next refresh fetch the group list again.
func invalidateJoinedGroups() {
	groupListMu.Lock()
	groupListCached = nil
	groupListMu.Unlock()
}

// syncFollowedChannelsThrottled is the refresh's channel sync, run at most
// once per followedChannelsEvery. Following or unfollowing a channel calls
// syncFollowedChannels directly.
func syncFollowedChannelsThrottled() {
	followedChannelsMu.Lock()
	if !followedChannelsLast.IsZero() && time.Since(followedChannelsLast) < followedChannelsEvery {
		followedChannelsMu.Unlock()
		return
	}
	followedChannelsLast = time.Now()
	followedChannelsMu.Unlock()
	syncFollowedChannels()
}
