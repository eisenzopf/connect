import { act, needClient, needConversation, value, journal, showPending } from "./app.js";
const byId=id=>document.getElementById(id);
let lookupRevision=0;
let selectedParticipant=null;
function clearSelection() {
    lookupRevision++;
    // Do not erase a participant the developer entered independently.
    if(selectedParticipant!==null && byId("participant").value===selectedParticipant)
        byId("participant").value="";
    selectedParticipant=null;
    byId("contact-state").textContent="";
    byId("contact-routes").replaceChildren();
}
for(const id of ["contact-address","conversation","participant"])
    byId(id).addEventListener("input",clearSelection);
byId("inspect-contact").onclick=()=>act("Contact inspected",async()=>{
    clearSelection();
    const revision=lookupRevision;
    const address=value("contact-address"),conversation=value("conversation");
    const participant=value("participant");
    const current=()=>{
        if(revision!==lookupRevision || address!==value("contact-address") ||
            conversation!==value("conversation") || participant!==value("participant"))
            throw Error("Contact selection changed; inspect it again.");
    };
    const client=needClient();const identity=await client.resolve(address);
    current();
    const capabilities=await client.capabilities(address,conversation||null);
    current();
    const ownership=await client.contactState(identity.id);
    current();
    const availability=capabilities.availability;
    const fresh=availability?.expires_at>Math.floor(Date.now()/1000);
    const rows=capabilities.routes.map(route=>{
        const row=document.createElement("li");row.textContent=`${route.transport} · ${route.media.join(", ")} · ${route.readiness}`;return row;
    });
    // Publish one completed selection, never a partial result across awaits.
    byId("participant").value=identity.id;
    selectedParticipant=identity.id;
    byId("contact-state").textContent=`Native ownership: ${ownership.ownership}. Availability: ${fresh?availability.state:"unknown or stale"}. Pairing does not establish SMS enrollment.`;
    const routes=byId("contact-routes");routes.replaceChildren();
    for(const row of rows) routes.append(row);
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
