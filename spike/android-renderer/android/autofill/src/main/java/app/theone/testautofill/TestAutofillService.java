package app.theone.testautofill;

import android.app.assist.AssistStructure;
import android.os.CancellationSignal;
import android.service.autofill.AutofillService;
import android.service.autofill.Dataset;
import android.service.autofill.Field;
import android.service.autofill.FillCallback;
import android.service.autofill.FillRequest;
import android.service.autofill.FillResponse;
import android.service.autofill.Presentations;
import android.service.autofill.SaveCallback;
import android.service.autofill.SaveInfo;
import android.service.autofill.SaveRequest;
import android.text.InputType;
import android.util.Log;
import android.view.View;
import android.view.autofill.AutofillId;
import android.view.autofill.AutofillValue;
import android.widget.RemoteViews;
import java.util.ArrayList;
import java.util.List;

/** Offers one login, "ada@example.com" and "correct horse", for a form's username and password fields, found by
 * their autofill hints or, failing that, a password input type. Logs every field it was shown and what it's asked
 * to save, so a check sees what the app told Android. */
public class TestAutofillService extends AutofillService {
    private static final String TAG = "TestAutofill";

    @Override public void onFillRequest(FillRequest request, CancellationSignal cancel, FillCallback callback) {
        AssistStructure structure = request.getFillContexts().get(request.getFillContexts().size() - 1).getStructure();
        List<AssistStructure.ViewNode> nodes = new ArrayList<>();
        for (int i = 0; i < structure.getWindowNodeCount(); i++) walk(structure.getWindowNodeAt(i).getRootViewNode(), nodes);
        AutofillId username = null, password = null;
        for (AssistStructure.ViewNode node : nodes) {
            String[] hints = node.getAutofillHints();
            String hint = hints != null && hints.length > 0 ? hints[0] : "";
            boolean secret = (node.getInputType() & InputType.TYPE_MASK_VARIATION) == InputType.TYPE_TEXT_VARIATION_PASSWORD;
            Log.i(TAG, "field " + node.getAutofillId() + " hints=" + hint + " password=" + secret + " hint=" + node.getHint() + " class=" + node.getClassName());
            if (hint.equals(View.AUTOFILL_HINT_PASSWORD) || (hint.isEmpty() && secret)) password = node.getAutofillId();
            else if (hint.equals(View.AUTOFILL_HINT_USERNAME) || hint.equals(View.AUTOFILL_HINT_EMAIL_ADDRESS)) username = node.getAutofillId();
        }
        if (username == null && password == null) {
            callback.onSuccess(null);
            return;
        }
        RemoteViews row = new RemoteViews(getPackageName(), android.R.layout.simple_list_item_1);
        row.setTextViewText(android.R.id.text1, "Test autofill: ada@example.com");
        Presentations presentations = new Presentations.Builder().setMenuPresentation(row).build();
        Dataset.Builder dataset = new Dataset.Builder(presentations);
        if (username != null) dataset.setField(username, new Field.Builder().setValue(AutofillValue.forText("ada@example.com")).build());
        if (password != null) dataset.setField(password, new Field.Builder().setValue(AutofillValue.forText("correct horse")).build());
        FillResponse.Builder response = new FillResponse.Builder().addDataset(dataset.build());
        List<AutofillId> ids = new ArrayList<>();
        if (username != null) ids.add(username);
        if (password != null) ids.add(password);
        response.setSaveInfo(new SaveInfo.Builder(SaveInfo.SAVE_DATA_TYPE_USERNAME | SaveInfo.SAVE_DATA_TYPE_PASSWORD, ids.toArray(new AutofillId[0])).build());
        callback.onSuccess(response.build());
    }

    @Override public void onSaveRequest(SaveRequest request, SaveCallback callback) {
        AssistStructure structure = request.getFillContexts().get(request.getFillContexts().size() - 1).getStructure();
        List<AssistStructure.ViewNode> nodes = new ArrayList<>();
        for (int i = 0; i < structure.getWindowNodeCount(); i++) walk(structure.getWindowNodeAt(i).getRootViewNode(), nodes);
        for (AssistStructure.ViewNode node : nodes) {
            AutofillValue value = node.getAutofillValue();
            if (value != null && value.isText()) Log.i(TAG, "save " + node.getAutofillId() + " length=" + value.getTextValue().length());
        }
        callback.onSuccess();
    }

    private static void walk(AssistStructure.ViewNode node, List<AssistStructure.ViewNode> out) {
        if (node.getAutofillType() == View.AUTOFILL_TYPE_TEXT) out.add(node);
        for (int i = 0; i < node.getChildCount(); i++) walk(node.getChildAt(i), out);
    }
}
