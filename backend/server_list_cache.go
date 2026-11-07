package main

import (
	"context"
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
//
// The group list keeps for hours because every change to it is announced: a
// group notification, or the "groups" dirty bit for ones missed while
// offline. The official apps ask for the whole list only on that dirty bit
// (WAWebHandleDirtyBits), never just for having reconnected, so neither does
// a reconnect here. The age limit is only a backstop for a missed
// announcement.
const (
	groupListMaxAge       = 6 * time.Hour
	followedChannelsEvery = time.Minute
)

func (acc *Account) serverGroupList() ([]*types.GroupInfo, error) {
	acc.groupListMu.Lock()
	defer acc.groupListMu.Unlock()
	if acc.groupListCached != nil && time.Since(acc.groupListFetched) < groupListMaxAge {
		return acc.groupListCached, nil
	}
	groups, err := acc.wa().GetJoinedGroups(context.Background())
	if err != nil {
		return nil, err
	}
	acc.groupListCached, acc.groupListFetched = groups, time.Now()
	return groups, nil
}

func (acc *Account) invalidateJoinedGroups() {
	acc.groupListMu.Lock()
	acc.groupListCached = nil
	acc.groupListMu.Unlock()
}

func (acc *Account) syncFollowedChannelsThrottled() {
	acc.followedChannelsMu.Lock()
	if !acc.followedChannelsLast.IsZero() && time.Since(acc.followedChannelsLast) < followedChannelsEvery {
		acc.followedChannelsMu.Unlock()
		return
	}
	acc.followedChannelsLast = time.Now()
	acc.followedChannelsMu.Unlock()
	acc.syncFollowedChannels()
}
