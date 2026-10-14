import { act, needClient, needConversation, value, journal, showPending } from "./app.js";
const byId=id=>document.getElementById(id);
byId("inspect-contact").onclick=()=>act("Contact inspected",async()=>{
    const client=needClient();const identity=await client.resolve(value("contact-address"));
    byId("participant").value=identity.id;
    const capabilities=await client.capabilities(value("contact-address"),value("conversation")||null);
    const ownership=await client.contactState(identity.id);
    const availability=capabilities.availability;
    const fresh=availability?.expires_at>Math.floor(Date.now()/1000);
    byId("contact-state").textContent=`Native ownership: ${ownership.ownership}. Availability: ${fresh?availability.state:"unknown or stale"}. Pairing does not establish SMS enrollment.`;
    const routes=byId("contact-routes");routes.replaceChildren();
    for(const route of capabilities.routes) {
        const row=document.createElement("li");row.textContent=`${route.transport} · ${route.media.join(", ")} · ${route.readiness}`;routes.append(row);
    }
});
byId("pair-start").onclick=()=>act("Pairing started",async()=>{
    const slot=`pair-start:${value("participant")}`;
    const retained=journal().read()[slot];
    const result=await journal().runPairing(needClient(),slot,{op:"contact_pairing_start",
        peer_identity_id:value("participant"),expires_at:retained?.input.expires_at??Math.floor(Date.now()/1000)+600});
    byId("pairing-id").value=result.data.id;byId("pairing-challenge").value=result.data.challenge??"";showPending();
});
byId("pair-complete").onclick=()=>act("Pairing completed",async()=>{
    await journal().runPairing(needClient(),`pair-complete:${value("pairing-id")}`,{op:"contact_pairing_complete",
        pairing_id:value("pairing-id"),challenge:value("pairing-challenge")});
    byId("pairing-challenge").value="";showPending();
});
byId("pair-revoke").onclick=()=>act("Pairing revoked",async()=>{
    await journal().runPairing(needClient(),`pair-revoke:${value("pairing-id")}`,{op:"contact_pairing_revoke",pairing_id:value("pairing-id")});
    byId("pairing-challenge").value="";showPending();
});
