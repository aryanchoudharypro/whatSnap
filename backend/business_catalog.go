package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
)

type CatalogRequest struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
}

const (
	catalogPageSize    = 50
	catalogMaxProducts = 300
)

func formatPrice(p *whatsmeow.CatalogPrice) string {
	if p == nil {
		return ""
	}
	return formatAmount1000(p.Amount1000, p.Currency)
}

func formatAmount1000(amount int64, currency string) string {
	sign := ""
	if amount < 0 {
		sign, amount = "-", -amount
	}
	cents := (amount + 5) / 10
	text := fmt.Sprintf("%s%d.%02d", sign, cents/100, cents%100)
	if currency != "" {
		text = currency + " " + text
	}
	return text
}

func availabilityText(a string) string {
	switch a {
	case "", "IN_STOCK":
		return ""
	case "OUT_OF_STOCK":
		return "out of stock"
	case "AVAILABLE_FOR_ANOTHER_POSTCODE":
		return "not available for your postcode"
	}
	return strings.ToLower(strings.ReplaceAll(a, "_", " "))
}

func (acc *Account) productLink(business types.JID, productID string) string {
	pn := business
	if pn.Server == types.HiddenUserServer {
		if mapped, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), business); err == nil && !mapped.IsEmpty() {
			pn = mapped
		} else {
			return ""
		}
	}
	if pn.Server != types.DefaultUserServer || productID == "" {
		return ""
	}
	return "https://wa.me/p/" + productID + "/" + pn.User
}

func (acc *Account) handleGetCatalog(conn net.Conn, raw json.RawMessage) {
	var data CatalogRequest
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { acc.sendEvent(conn, "error", map[string]string{"message": msg}) }
		business, err := types.ParseJID(data.JID)
		select_ := ""
		if data.MessageID != "" {
			if msg, ok := acc.loadMessageProto(data.MessageID); ok {
				if product := unwrapMessage(msg).GetProductMessage(); product != nil {
					select_ = product.GetProduct().GetProductID()
					if owner, ownerErr := types.ParseJID(product.GetBusinessOwnerJID()); ownerErr == nil && owner.User != "" {
						business, err = owner, nil
					}
				}
			}
		}
		if err != nil || business.User == "" {
			fail("This chat has no catalogue.")
			return
		}
		business = business.ToNonAD()
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		var products []map[string]interface{}
		after := ""
		for len(products) < catalogMaxProducts {
			page, err := acc.wa().GetBusinessCatalog(ctx, business, catalogPageSize, after)
			if err != nil {
				if len(products) > 0 {
					break
				}
				log.Printf("catalogue of %s: %v", business, err)
				fail("Couldn't load the catalogue. This business may not have one.")
				return
			}
			for _, p := range page.Products {
				if p.Hidden {
					continue
				}
				item := map[string]interface{}{
					"id":           p.ID,
					"name":         cmpOr(strings.TrimSpace(p.Name), "Unnamed product"),
					"description":  strings.TrimSpace(p.Description),
					"price":        formatPrice(p.Price),
					"availability": availabilityText(p.Availability),
					"url":          p.URL,
					"retailer_id":  p.RetailerID,
					"link":         acc.productLink(business, p.ID),
				}
				if p.SalePrice != nil {
					item["sale_price"] = formatPrice(p.SalePrice)
					if p.SaleStart != "" {
						item["sale_dates"] = p.SaleStart + " to " + p.SaleEnd
					}
				}
				products = append(products, item)
			}
			after = page.After
			if after == "" || len(page.Products) == 0 {
				break
			}
		}
		if len(products) == 0 {
			fail("This business's catalogue is empty.")
			return
		}
		acc.sendEvent(conn, "catalog", map[string]interface{}{
			"jid": business.String(), "products": products, "select": select_, "more": after != "",
		})
	}()
}

func (acc *Account) handleGetOrder(conn net.Conn, raw json.RawMessage) {
	var data CatalogRequest
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { acc.sendEvent(conn, "error", map[string]string{"message": msg}) }
		msg, ok := acc.loadMessageProto(data.MessageID)
		order := (*waE2E.OrderMessage)(nil)
		if ok {
			order = unwrapMessage(msg).GetOrderMessage()
		}
		if order == nil || order.GetOrderID() == "" || order.GetToken() == "" {
			fail("This order's details can't be opened here.")
			return
		}
		seller, err := types.ParseJID(order.GetSellerJID())
		if err != nil || seller.User == "" {
			seller, _ = types.ParseJID(data.JID)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		details, err := acc.wa().GetBusinessOrder(ctx, seller, order.GetOrderID(), order.GetToken())
		if err != nil {
			if old, oldErr := acc.wa().GetOrderDetails(ctx, order.GetOrderID(), order.GetToken()); oldErr == nil {
				details = orderFromIQ(old)
				err = nil
			}
		}
		if err != nil {
			log.Printf("order %s: %v", order.GetOrderID(), err)
			fail("Couldn't load the order's details.")
			return
		}
		acc.sendEvent(conn, "order_details", map[string]interface{}{
			"title": cmpOr(strings.TrimSpace(order.GetOrderTitle()), "Order"),
			"text":  orderText(order, details),
		})
	}()
}

func orderFromIQ(o *types.OrderDetails) *whatsmeow.BusinessOrder {
	out := &whatsmeow.BusinessOrder{Currency: o.Price.Currency, CreatedAt: o.CreatedAt.Unix()}
	out.Subtotal = &whatsmeow.CatalogPrice{Amount1000: o.Price.Subtotal, Currency: o.Price.Currency}
	out.Total = &whatsmeow.CatalogPrice{Amount1000: o.Price.Total, Currency: o.Price.Currency}
	for _, p := range o.Products {
		item := whatsmeow.BusinessOrderItem{
			ID: p.ID, Name: p.Name, Quantity: int64(p.Quantity),
			Price: &whatsmeow.CatalogPrice{Amount1000: p.Price, Currency: p.Currency},
		}
		if p.VariantInfo.Properties != "" {
			item.Variant = [][2]string{{"", p.VariantInfo.Properties}}
		}
		out.Items = append(out.Items, item)
	}
	return out
}

func orderText(order *waE2E.OrderMessage, d *whatsmeow.BusinessOrder) string {
	var lines []string
	if title := strings.TrimSpace(order.GetOrderTitle()); title != "" {
		lines = append(lines, title)
	}
	if d.CreatedAt > 0 {
		lines = append(lines, "Placed "+time.Unix(d.CreatedAt, 0).Format("Monday 2 January 2006, 15:04"))
	}
	if status := order.GetStatus(); status != 0 {
		lines = append(lines, "Status: "+strings.ToLower(status.String()))
	}
	if note := strings.TrimSpace(order.GetMessage()); note != "" {
		lines = append(lines, "Note: "+note)
	}
	lines = append(lines, "")
	for _, item := range d.Items {
		line := fmt.Sprintf("%d x %s", max(item.Quantity, 1), cmpOr(item.Name, "Item"))
		var variant []string
		for _, kv := range item.Variant {
			if kv[0] != "" {
				variant = append(variant, kv[0]+": "+kv[1])
			} else if kv[1] != "" {
				variant = append(variant, kv[1])
			}
		}
		if len(variant) > 0 {
			line += " (" + strings.Join(variant, ", ") + ")"
		}
		if price := formatPrice(item.Price); price != "" {
			line += ", " + price + " each"
		}
		lines = append(lines, line)
	}
	lines = append(lines, "")
	if s := formatPrice(d.Subtotal); s != "" {
		lines = append(lines, "Subtotal: "+s)
	}
	if t := formatPrice(d.Total); t != "" {
		lines = append(lines, "Total: "+t)
	} else if order.GetTotalAmount1000() > 0 {
		lines = append(lines, "Total: "+formatAmount1000(order.GetTotalAmount1000(), order.GetTotalCurrencyCode()))
	}
	return strings.TrimSpace(strings.Join(lines, "\n"))
}
