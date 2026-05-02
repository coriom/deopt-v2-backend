import requests

def envoyer_message(token, chat_id, texte):
    url = f"https://api.telegram.org/bot{token}/sendMessage"
    payload = {"chat_id": chat_id, "text": texte}
    
    reponse = requests.post(url, data=payload)
    return reponse.json()

MON_TOKEN = "8609690384:AAF0kWFUJ69QLibL9ERhediBIcvTevg67lo"
MON_CHAT_ID = "6156557608"
MESSAGE = "done"

envoyer_message(MON_TOKEN, MON_CHAT_ID, MESSAGE)
